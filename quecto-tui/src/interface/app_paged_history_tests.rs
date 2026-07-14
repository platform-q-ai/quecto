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

    h.app_mut().handle_key(Key::PageUp);
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
    let a = h.app_mut();
    respond(
        a,
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        page(
            &[("m3", "third message"), ("m4", "fourth message")],
            Some("m3"),
            true,
        ),
    );

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
async fn stubbed_history_message_can_be_recalled_by_reference() {
    let mut h = harness().await;
    respond(
        h.app_mut(),
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        serde_json::json!({
            "messages": [{
                "id": "stub-1",
                "role": "assistant",
                "content": "[content stub — recall available]",
            }],
            "hasMoreBefore": false,
            "before": null,
        }),
    );
    let _ = h.drain_commands().await;

    h.app_mut()
        .request_history_message_recall_for_test("stub-1");
    let commands = h.drain_commands().await;
    let get_message = commands.iter().find_map(|line| {
        let cmd = serde_json::from_str::<serde_json::Value>(line).ok()?;
        (cmd.get("type").and_then(|v| v.as_str()) == Some("get_message")).then_some(cmd)
    });

    assert_eq!(
        get_message
            .as_ref()
            .and_then(|cmd| cmd.get("messageId"))
            .and_then(|v| v.as_str()),
        Some("stub-1"),
        "TUI must be able to request full content for a stubbed history message; commands={commands:?}"
    );
}
