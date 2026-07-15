use super::app_response::ATTACH_BACKFILL_ID;
use super::tui_harness::TuiHarness;
use super::*;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

fn respond(app: &mut App, id: Option<&str>, command: &str, success: bool, data: serde_json::Value) {
    app.handle_response(
        id.map(String::from),
        command.to_string(),
        success,
        Some(data),
        None,
    );
}

fn page(
    messages: &[(&str, &str)],
    before: Option<&str>,
    has_more_before: bool,
) -> serde_json::Value {
    serde_json::json!({
        "messages": messages
            .iter()
            .map(|(id, content)| serde_json::json!({
                "id": id,
                "role": "user",
                "content": content,
            }))
            .collect::<Vec<_>>(),
        "before": before,
        "hasMoreBefore": has_more_before,
    })
}

fn chat_text(app: &mut App) -> String {
    app.master_session
        .chat
        .render(120)
        .iter()
        .map(|line| super::app_methods::strip_ansi(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn child_chat_text(app: &mut App, agent_id: &str) -> String {
    app.subagents
        .sessions
        .get_mut(agent_id)
        .expect("child session exists")
        .chat
        .render(120)
        .iter()
        .map(|line| super::app_methods::strip_ansi(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn prime_active_viewport(app: &mut App) {
    let chat = app.active_chat_mut();
    chat.set_viewport_height(1);
    let _ = chat.render(120);
}

/// Undo `prime_active_viewport` so `chat_text` captures the whole transcript
/// again (a 1-line scroll viewport would otherwise render a single line).
fn widen_active_viewport(app: &mut App) {
    let chat = app.active_chat_mut();
    chat.set_viewport_height(200);
    chat.scroll_down(usize::MAX);
}

async fn drained_get_messages_commands(h: &mut TuiHarness) -> Vec<serde_json::Value> {
    h.drain_commands()
        .await
        .into_iter()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(&line).ok())
        .filter(|cmd| cmd.get("type").and_then(|v| v.as_str()) == Some("get_messages"))
        .collect()
}

#[tokio::test]
async fn attach_backfill_requests_next_older_page_when_scrolled_to_top() {
    let mut h = harness().await;
    {
        let a = h.app_mut();
        respond(
            a,
            Some(ATTACH_BACKFILL_ID),
            "get_messages",
            true,
            page(&[("m3", "newest-page")], Some("m3"), true),
        );
    }
    let _ = h.drain_commands().await;
    prime_active_viewport(h.app_mut());

    h.app_mut().handle_key(Key::PageUp);
    let commands = drained_get_messages_commands(&mut h).await;

    assert!(
        commands.iter().any(|cmd| {
            cmd.get("before").and_then(|v| v.as_str()) == Some("m3")
                && cmd
                    .get("id")
                    .and_then(|v| v.as_str())
                    .is_some_and(|id| id.starts_with("history-page-"))
        }),
        "scroll-back must request the next older history page with the advertised cursor; commands={commands:?}"
    );
}

#[tokio::test]
async fn older_history_request_is_deduped_while_cursor_is_in_flight() {
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        page(&[("m3", "newest-page")], Some("m3"), true),
    );
    let _ = h.drain_commands().await;
    prime_active_viewport(h.app_mut());

    h.app_mut().handle_key(Key::PageUp);
    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let commands = drained_get_messages_commands(&mut h).await;

    assert_eq!(
        commands.len(),
        1,
        "a pending before cursor must suppress duplicate older-page requests; commands={commands:?}"
    );
    assert_eq!(
        commands[0].get("before").and_then(|v| v.as_str()),
        Some("m3")
    );
}

#[tokio::test]
async fn subagent_older_history_request_targets_active_child_session() {
    let mut h = harness().await;
    h.event(Event::SubagentStateChanged {
        subagents: vec![crate::infrastructure::client::SubagentInfoEvent {
            agent_id: "worker".into(),
            status: "idle".into(),
            last_tool: None,
            last_error: None,
            pid: 7,
            socket_path: Some("/tmp/worker.sock".into()),
            parent_id: None,
            workflow: None,
            read_only: false,
        }],
    });
    h.app_mut().select_agent(Some("worker"));
    {
        let session = h.app_mut().active_session_mut();
        session.history_has_more_before = true;
        session.history_before_cursor = Some("child-cursor".into());
        session.chat.add_entry(ChatEntry::User {
            text: "child".into(),
        });
        session.chat.set_viewport_height(1);
        let _ = session.chat.render(120);
        session.chat.scroll_up(usize::MAX);
    }

    let request = h
        .app_mut()
        .next_history_page_request()
        .expect("child page request is available");

    assert_eq!(request.1, "child-cursor");
    assert!(
        request.2,
        "selected sub-agent history must target the child connection"
    );
}

#[tokio::test]
async fn paged_resume_replaces_stale_chat_before_preserving_cursor() {
    let mut h = harness().await;
    h.app_mut().master_session.chat.add_entry(ChatEntry::User {
        text: "stale session A".into(),
    });

    respond(
        h.app_mut(),
        Some("resume-messages"),
        "get_messages",
        true,
        page(&[("m2", "resumed newest")], Some("m2"), true),
    );
    let frame = chat_text(h.app_mut());

    assert!(
        frame.contains("resumed newest"),
        "resumed history must render: {frame}"
    );
    assert!(
        frame.contains("Session resumed"),
        "resume status should remain visible: {frame}"
    );
    assert!(
        !frame.contains("stale session A"),
        "paged resume must replace the prior session transcript, not prepend into it: {frame}"
    );
    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let commands = drained_get_messages_commands(&mut h).await;
    assert!(
        commands
            .iter()
            .any(|cmd| cmd.get("before").and_then(|v| v.as_str()) == Some("m2")),
        "paged resume must still preserve the older-history cursor; commands={commands:?}"
    );
}

#[tokio::test]
async fn older_history_page_prepends_without_gap_or_duplicate() {
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        page(
            &[("m3", "third message"), ("m4", "fourth message")],
            Some("m3"),
            true,
        ),
    );
    // Issue the older-page request through the production scroll path: a page
    // response is only applied when it matches the client's own in-flight
    // request id exactly (#1061 review).
    let _ = h.drain_commands().await;
    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let _ = h.drain_commands().await;

    let a = h.app_mut();
    respond(
        a,
        Some("history-page-1"),
        "get_messages",
        true,
        page(
            &[("m1", "first message"), ("m2", "second message")],
            None,
            false,
        ),
    );
    widen_active_viewport(a);
    let frame = chat_text(a);

    let first = frame.find("first message").expect("oldest page rendered");
    let second = frame
        .find("second message")
        .expect("older page tail rendered");
    let third = frame
        .find("third message")
        .expect("newer page head rendered");
    let fourth = frame
        .find("fourth message")
        .expect("newest message rendered");
    assert!(
        first < second && second < third && third < fourth,
        "older page must join directly before the existing page without an interior gap:\n{frame}"
    );
    assert_eq!(
        frame.matches("third message").count(),
        1,
        "backfill must not duplicate the existing newest page:\n{frame}"
    );
}

#[tokio::test]
async fn resumed_paged_history_keeps_older_page_reachable() {
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some("resume-messages"),
        "get_messages",
        true,
        page(&[("m2", "resumed newest")], Some("m2"), true),
    );

    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let commands = drained_get_messages_commands(&mut h).await;

    assert!(
        commands
            .iter()
            .any(|cmd| cmd.get("before").and_then(|v| v.as_str()) == Some("m2")),
        "resume must preserve the paging cursor so older resumed history remains reachable; commands={commands:?}"
    );
}

#[tokio::test]
async fn stubbed_history_message_is_recalled_and_replaced_on_scroll() {
    let mut h = harness().await;
    // Attach delivers a ladder-demoted stub in place (collapsed=true).
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        serde_json::json!({
            "messages": [{
                "id": "stub-1",
                "role": "assistant",
                "content": "[assistant stub — recall available]",
                "collapsed": true,
            }],
            "hasMoreBefore": false,
            "before": null,
        }),
    );
    let _ = h.drain_commands().await;

    // Scrolling back auto-recalls the stub's full content through the production
    // key path — no test-only entry point.
    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let commands = h.drain_commands().await;
    let get_message = commands
        .iter()
        .find_map(|line| {
            let cmd = serde_json::from_str::<serde_json::Value>(line).ok()?;
            (cmd.get("type").and_then(|v| v.as_str()) == Some("get_message")).then_some(cmd)
        })
        .expect("scrolling a stub into view must auto-request its full content");
    assert_eq!(
        get_message.get("messageId").and_then(|v| v.as_str()),
        Some("stub-1")
    );
    let req_id = get_message
        .get("id")
        .and_then(|v| v.as_str())
        .expect("recall request carries a correlation id")
        .to_string();

    // Delivering the full body replaces the stub in place.
    respond(
        h.app_mut(),
        Some(&req_id),
        "get_message",
        true,
        serde_json::json!({
            "id": "stub-1",
            "role": "assistant",
            "content": "the full recalled answer",
        }),
    );
    // Re-anchor to the bottom (the PageUp above scrolled this tiny transcript
    // out of the no-viewport render window) so the recalled body is captured.
    h.app_mut().active_chat_mut().scroll_down(1000);
    let frame = chat_text(h.app_mut());
    assert!(
        frame.contains("the full recalled answer"),
        "recalled content must replace the stub: {frame}"
    );
    assert!(
        !frame.contains("recall available"),
        "the stub body must be gone after recall: {frame}"
    );
}

#[tokio::test]
async fn stub_recall_rejects_mismatched_role_and_does_not_retry() {
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        serde_json::json!({
            "messages": [{
                "id": "stub-role",
                "role": "assistant",
                "content": "[assistant stub — recall available]",
                "collapsed": true,
            }],
            "hasMoreBefore": false,
            "before": null,
        }),
    );
    let _ = h.drain_commands().await;

    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let commands = h.drain_commands().await;
    let req_id = commands
        .iter()
        .find_map(|line| {
            let cmd: serde_json::Value = serde_json::from_str(line).ok()?;
            (cmd.get("type").and_then(|v| v.as_str()) == Some("get_message"))
                .then(|| cmd.get("id")?.as_str().map(str::to_owned))?
        })
        .expect("visible stub must issue recall");

    respond(
        h.app_mut(),
        Some(&req_id),
        "get_message",
        true,
        serde_json::json!({
            "id": "stub-role",
            "role": "user",
            "content": "must not replace assistant stub",
        }),
    );
    h.app_mut().active_chat_mut().scroll_down(1000);
    let frame = chat_text(h.app_mut());
    assert!(
        frame.contains("recall available"),
        "role mismatch must preserve stub: {frame}"
    );
    assert!(
        !frame.contains("must not replace"),
        "role mismatch must reject content: {frame}"
    );

    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    assert!(
        h.drain_commands().await.iter().all(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|cmd| cmd.get("type").and_then(|v| v.as_str()).map(str::to_owned))
                .as_deref()
                != Some("get_message")
        }),
        "a rejected permanent response must not retry on the next scroll"
    );
}

#[tokio::test]
async fn failed_stub_recall_does_not_retry_on_every_scroll() {
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        serde_json::json!({
            "messages": [{
                "id": "missing-stub",
                "role": "assistant",
                "content": "[assistant stub — recall available]",
                "collapsed": true,
            }],
            "hasMoreBefore": false,
            "before": null,
        }),
    );
    let _ = h.drain_commands().await;

    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let commands = h.drain_commands().await;
    let req_id = commands
        .iter()
        .find_map(|line| {
            let cmd: serde_json::Value = serde_json::from_str(line).ok()?;
            (cmd.get("type").and_then(|v| v.as_str()) == Some("get_message"))
                .then(|| cmd.get("id")?.as_str().map(str::to_owned))?
        })
        .expect("visible stub must issue recall");

    h.app_mut().handle_response(
        Some(req_id),
        "get_message".into(),
        false,
        None,
        Some("message not found".into()),
    );
    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    assert!(
        h.drain_commands().await.iter().all(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .and_then(|cmd| cmd.get("type").and_then(|v| v.as_str()).map(str::to_owned))
                .as_deref()
                != Some("get_message")
        }),
        "a permanent get_message failure must not retry on every scroll"
    );
}

#[tokio::test]
async fn subagent_older_history_page_prepends_without_replacing_newest() {
    let mut h = harness().await;
    h.event(Event::SubagentStateChanged {
        subagents: vec![crate::infrastructure::client::SubagentInfoEvent {
            agent_id: "worker".into(),
            status: "idle".into(),
            last_tool: None,
            last_error: None,
            pid: 7,
            socket_path: Some("/tmp/worker.sock".into()),
            parent_id: None,
            workflow: None,
            read_only: false,
        }],
    });
    h.select(Some("worker"));

    // Initial (newest) child page reports older history before it.
    h.route(
        "worker",
        Event::Response {
            id: Some("child-backfill".into()),
            command: "get_messages".into(),
            success: true,
            data: Some(page(
                &[("m3", "third message"), ("m4", "fourth message")],
                Some("m3"),
                true,
            )),
            error: None,
        },
    );
    // Register the older-page request through the production request builder so
    // the response below correlates with the child's own in-flight id (#1061
    // review: uncorrelated pages are dropped).
    {
        let session = h.app_mut().active_session_mut();
        session.chat.set_viewport_height(1);
        let _ = session.chat.render(120);
        session.chat.scroll_up(usize::MAX);
    }
    let request = h
        .app_mut()
        .next_history_page_request()
        .expect("child older-page request is available");
    assert_eq!(request.0, "history-page-1");
    // The explicitly-requested older page must PREPEND, not replace the newest
    // page (regression: the child reconciler used replace_history_prefix).
    h.route(
        "worker",
        Event::Response {
            id: Some("history-page-1".into()),
            command: "get_messages".into(),
            success: true,
            data: Some(page(
                &[("m1", "first message"), ("m2", "second message")],
                None,
                false,
            )),
            error: None,
        },
    );

    let frame = child_chat_text(h.app_mut(), "worker");
    let first = frame.find("first message").expect("oldest page rendered");
    let second = frame
        .find("second message")
        .expect("older page tail rendered");
    let third = frame
        .find("third message")
        .expect("newer page head rendered");
    let fourth = frame
        .find("fourth message")
        .expect("newest message rendered");
    assert!(
        first < second && second < third && third < fourth,
        "child older page must join before the existing page without a gap:\n{frame}"
    );
    assert_eq!(
        frame.matches("third message").count(),
        1,
        "child backfill must not duplicate or replace the newest page:\n{frame}"
    );
}

#[tokio::test]
async fn failed_older_page_clears_cursor_and_allows_retry() {
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        page(&[("m3", "newest-page")], Some("m3"), true),
    );
    let _ = h.drain_commands().await;

    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let first = drained_get_messages_commands(&mut h).await;
    assert_eq!(first.len(), 1, "one older-page request should be in flight");

    // The older-page request fails transiently.
    h.app_mut().handle_response(
        Some("history-page-1".into()),
        "get_messages".into(),
        false,
        None,
        Some("transient error".into()),
    );

    // A subsequent scroll must be able to retry the SAME cursor (the failed
    // request must not leave it permanently in flight).
    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp);
    let retry = drained_get_messages_commands(&mut h).await;
    assert!(
        retry
            .iter()
            .any(|cmd| cmd.get("before").and_then(|v| v.as_str()) == Some("m3")),
        "a failed older page must be retryable on the next scroll; commands={retry:?}"
    );
}

#[tokio::test]
async fn history_page_with_foreign_correlation_id_is_dropped() {
    // get_messages responses are broadcast to every connected client: another
    // client's older page is paged from a DIFFERENT depth, so applying it here
    // would create an interior gap (#1061 review).
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        page(
            &[("m3", "third message"), ("m4", "fourth message")],
            Some("m3"),
            true,
        ),
    );
    let _ = h.drain_commands().await;
    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp); // our in-flight id: history-page-1
    let _ = h.drain_commands().await;

    respond(
        h.app_mut(),
        Some("history-page-9"),
        "get_messages",
        true,
        page(&[("x1", "foreign client page")], None, false),
    );
    widen_active_viewport(h.app_mut());
    let frame = chat_text(h.app_mut());
    assert!(
        !frame.contains("foreign client page"),
        "a page that does not match our in-flight request id must be dropped:\n{frame}"
    );

    // Our own in-flight page must still apply after the foreign one was dropped.
    respond(
        h.app_mut(),
        Some("history-page-1"),
        "get_messages",
        true,
        page(&[("m1", "first message")], None, false),
    );
    widen_active_viewport(h.app_mut());
    let frame = chat_text(h.app_mut());
    assert!(
        frame.contains("first message"),
        "our own correlated page must still prepend:\n{frame}"
    );
}

#[tokio::test]
async fn history_page_in_flight_across_resume_is_not_applied() {
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        page(&[("m9", "old session newest")], Some("m9"), true),
    );
    let _ = h.drain_commands().await;
    prime_active_viewport(h.app_mut());
    h.app_mut().handle_key(Key::PageUp); // in flight when the resume lands
    let _ = h.drain_commands().await;

    respond(
        h.app_mut(),
        Some("resume-messages"),
        "get_messages",
        true,
        page(&[("r1", "resumed newest")], None, false),
    );
    // The old conversation's page arrives late: it must not prepend into the
    // resumed transcript (#1061 review).
    respond(
        h.app_mut(),
        Some("history-page-1"),
        "get_messages",
        true,
        page(&[("m8", "old session older")], None, false),
    );

    widen_active_viewport(h.app_mut());
    let frame = chat_text(h.app_mut());
    assert!(
        frame.contains("resumed newest"),
        "resumed page renders: {frame}"
    );
    assert!(
        !frame.contains("old session older"),
        "a page orphaned by resume must not prepend into the new transcript:\n{frame}"
    );
}

// The rewind-refresh paging-state reset (#1061 review) is covered in
// `app_rewind_response_tests::rewind_refresh_replaces_transcript_and_resets_paging_state`.
