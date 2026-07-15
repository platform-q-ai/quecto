//! Step definitions for `tui_paged_history.feature` (#1061 / ADR-0008 part 3).
//!
//! These drive the REAL TUI backfill/scroll/recall paths through the headless
//! render harness (`quecto_tui::interface::app::tui_harness`) — the same App the
//! binary runs. Pages are delivered as the wire `get_messages`/`get_message`
//! responses the server would send, and assertions read observable state: the
//! rendered transcript and the commands the client emits.

use super::*;
use quecto_tui::infrastructure::client::Event;
use quecto_tui::interface::app::tui_harness::{
    TuiHarness, spawn_start, spawn_subagent_socket_with_commands, subagent_with_socket,
    subagents_changed,
};
use quecto_tui::interface::keys::Key;

const CHILD: &str = "worker";

/// Per-scenario paged-history fixture stored on the World.
#[derive(Debug, Default)]
pub struct PagedHistoryState {
    /// Full chronological (id, content) history the server would page over.
    messages: Vec<(String, String)>,
    page_size: usize,
    /// Whether a scroll-back step observed an older-history request.
    requested_older: bool,
    /// Commands decoded by the active child's real Unix socket.
    child_commands: Option<tokio::sync::mpsc::Receiver<String>>,
    stub_id: Option<String>,
    stub_full: Option<String>,
    expected_send_failures: usize,
    observed_send_failures: usize,
}

// ── harness plumbing ────────────────────────────────────────────────────────

fn init_harness(world: &mut TuiWorld) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let h = rt.block_on(TuiHarness::new());
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(h));
}

/// Run a closure against the harness inside the runtime context (key/select
/// paths spawn background tasks, so they need a live runtime).
fn drive<R>(world: &mut TuiWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("harness runtime")
        .handle()
        .clone();
    let _guard = handle.enter();
    let h = &mut world.tui_parity.as_mut().expect("harness").0;
    f(h)
}

/// Drain the master client's outgoing commands (no enter guard: `block_on`
/// cannot run inside an active runtime context).
fn drain(world: &mut TuiWorld) -> Vec<String> {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("harness runtime")
        .handle()
        .clone();
    let h = &mut world.tui_parity.as_mut().expect("harness").0;
    handle.block_on(h.drain_commands())
}

fn active_chat_text(world: &mut TuiWorld) -> String {
    drive(world, |h| h.active_chat_text(120))
}

// ── paging helpers (mirror the server's messages_page_json) ─────────────────

fn build_messages(n: usize) -> Vec<(String, String)> {
    (0..n)
        .map(|i| (format!("m{i}"), format!("message {i}")))
        .collect()
}

fn page_json(
    messages: &[(String, String)],
    page_size: usize,
    before: Option<&str>,
) -> serde_json::Value {
    let end = before
        .and_then(|cursor| messages.iter().position(|(id, _)| id == cursor))
        .unwrap_or(messages.len());
    let start = end.saturating_sub(page_size);
    let slice: Vec<serde_json::Value> = messages[start..end]
        .iter()
        .map(|(id, content)| serde_json::json!({"id": id, "role": "user", "content": content}))
        .collect();
    let has_more = start > 0;
    serde_json::json!({
        "messages": slice,
        "before": has_more.then(|| messages[start].0.clone()),
        "hasMoreBefore": has_more,
    })
}

fn get_messages_response(id: &str, data: serde_json::Value) -> Event {
    Event::Response {
        id: Some(id.into()),
        command: "get_messages".into(),
        success: true,
        data: Some(data),
        error: None,
    }
}

/// Newest cursor after the initial page (the id of the first message the initial
/// page omitted), or `None` if the whole history fit in one page.
fn newest_cursor(messages: &[(String, String)], page_size: usize) -> Option<String> {
    (messages.len() > page_size).then(|| messages[messages.len() - page_size].0.clone())
}

/// Latest older-page request as (correlation id, before cursor). The reply must
/// echo the request's OWN id: the client applies a page only when the response
/// id matches its in-flight request exactly (#1061 review).
fn find_older_request(cmds: &[String]) -> Option<(String, String)> {
    cmds.iter().rev().find_map(|line| {
        let v: serde_json::Value = serde_json::from_str(line).ok()?;
        if v.get("type").and_then(|t| t.as_str()) != Some("get_messages") {
            return None;
        }
        let before = v.get("before").and_then(|b| b.as_str())?.to_string();
        let id = v.get("id").and_then(|i| i.as_str())?.to_string();
        Some((id, before))
    })
}

fn deliver_master_page(world: &mut TuiWorld, id: &str, before: Option<&str>) {
    let page = page_json(&world.tui_paged.messages, world.tui_paged.page_size, before);
    drive(world, |h| {
        h.event(get_messages_response(id, page));
    });
}

/// Press PageUp, then serve every older page the client requests until the
/// beginning of history is reached (master session).
fn scroll_master_to_beginning(world: &mut TuiWorld) {
    let _ = drain(world);
    for _ in 0..64 {
        drive(world, |h| {
            h.press(Key::PageUp);
        });
        let cmds = drain(world);
        match find_older_request(&cmds) {
            Some((request_id, cursor)) => {
                world.tui_paged.requested_older = true;
                deliver_master_page(world, &request_id, Some(&cursor));
            }
            None => break,
        }
    }
}

// ── Given ───────────────────────────────────────────────────────────────────

#[given("a running agent session with prior conversation history")]
fn given_running_session_with_history(world: &mut TuiWorld) {
    init_harness(world);
    world.tui_paged = PagedHistoryState {
        messages: build_messages(7),
        page_size: 3,
        ..Default::default()
    };
}

#[given("the TUI is attached to a session with enough history to require backfill")]
fn given_attached_enough_history(world: &mut TuiWorld) {
    given_running_session_with_history(world);
    deliver_master_page(world, "attach-backfill", None);
}

#[given("the TUI is attached to a session whose history exactly fits in the initial backfill")]
fn given_attached_exact_fit(world: &mut TuiWorld) {
    init_harness(world);
    world.tui_paged = PagedHistoryState {
        messages: build_messages(3),
        page_size: 3,
        ..Default::default()
    };
    deliver_master_page(world, "attach-backfill", None);
}

#[given("the TUI is attached to a session with one older message beyond the initial backfill")]
fn given_attached_one_over(world: &mut TuiWorld) {
    init_harness(world);
    world.tui_paged = PagedHistoryState {
        messages: build_messages(4),
        page_size: 3,
        ..Default::default()
    };
    deliver_master_page(world, "attach-backfill", None);
}

#[given("a resumable session with enough history to require backfill")]
fn given_resumable_enough_history(world: &mut TuiWorld) {
    given_running_session_with_history(world);
}

#[given("the TUI is viewing a sub-agent with enough history to require backfill")]
fn given_viewing_subagent(world: &mut TuiWorld) {
    init_harness(world);
    world.tui_paged = PagedHistoryState {
        messages: build_messages(7),
        page_size: 3,
        ..Default::default()
    };
    let (socket, child_commands) = drive(world, |h| {
        h.event(Event::AgentStart);
        h.event(spawn_start(CHILD));
        let (socket, commands) = spawn_subagent_socket_with_commands(CHILD);
        h.event(subagents_changed(vec![subagent_with_socket(
            CHILD,
            "running",
            Some(("active", 0, 3)),
            Some(socket.clone()),
        )]));
        h.select(Some(CHILD));
        (socket, commands)
    });
    assert!(socket.exists(), "recording child socket should be bound");
    world.tui_paged.child_commands = Some(child_commands);
    // Newest child page, with older history advertised.
    let page = page_json(&world.tui_paged.messages, world.tui_paged.page_size, None);
    drive(world, |h| {
        h.route(CHILD, get_messages_response("child-backfill", page));
    });
    world.tui_viewed_agent = Some(CHILD.into());
}

#[given("the TUI is attached to a session containing a stubbed long message")]
fn given_attached_stub(world: &mut TuiWorld) {
    init_harness(world);
    world.tui_paged = PagedHistoryState {
        page_size: 3,
        stub_id: Some("stub-1".into()),
        stub_full: Some("the full demoted answer".into()),
        ..Default::default()
    };
    let page = serde_json::json!({
        "messages": [
            {"id": "u0", "role": "user", "content": "a question"},
            {
                "id": "stub-1",
                "role": "assistant",
                "content": "[assistant stub — recall available]",
                "collapsed": true,
            },
        ],
        "before": null,
        "hasMoreBefore": false,
    });
    drive(world, |h| {
        h.event(get_messages_response("attach-backfill", page));
    });
}

#[given("the TUI master command channel disconnects with older history available")]
fn given_disconnected_with_older_history(world: &mut TuiWorld) {
    world.tui_paged = PagedHistoryState {
        messages: build_messages(2),
        page_size: 1,
        expected_send_failures: 2,
        ..Default::default()
    };
    init_harness(world);
    let page = page_json(&world.tui_paged.messages, 1, None);
    drive(world, |h| {
        h.event(get_messages_response("attach-backfill", page));
        h.disconnect_master_commands();
    });
}

#[given("the TUI master command channel disconnects with a visible history stub")]
fn given_disconnected_with_stub(world: &mut TuiWorld) {
    world.tui_paged = PagedHistoryState {
        stub_id: Some("retry-stub".into()),
        expected_send_failures: 2,
        ..Default::default()
    };
    init_harness(world);
    drive(world, |h| {
        h.event(get_messages_response(
            "attach-backfill",
            serde_json::json!({
                "messages": [{
                    "id": "retry-stub",
                    "role": "assistant",
                    "content": "[assistant stub — recall available]",
                    "collapsed": true,
                }],
                "before": null,
                "hasMoreBefore": false,
            }),
        ));
        h.disconnect_master_commands();
    });
}

// ── When ────────────────────────────────────────────────────────────────────

#[when("the TUI attaches to the session socket")]
fn when_tui_attaches(world: &mut TuiWorld) {
    deliver_master_page(world, "attach-backfill", None);
}

#[when("the operator scrolls back until the beginning of history is reached")]
fn when_scroll_until_beginning(world: &mut TuiWorld) {
    scroll_master_to_beginning(world);
}

#[when("the operator scrolls back to the top of history")]
fn when_scroll_top_history(world: &mut TuiWorld) {
    scroll_master_to_beginning(world);
}

#[when("the operator scrolls back to the top of the newest history")]
fn when_scroll_top_newest(world: &mut TuiWorld) {
    scroll_master_to_beginning(world);
}

#[when("the operator scrolls back until the beginning of that history is reached")]
fn when_scroll_child_until_beginning(world: &mut TuiWorld) {
    let messages = world.tui_paged.messages.clone();
    let page_size = world.tui_paged.page_size;
    let mut cursor = newest_cursor(&messages, page_size);
    while let Some(current) = cursor {
        // Drive the production key/routing path, then require the real child
        // socket to observe the exact request before returning a page.
        drive(world, |h| {
            h.press(Key::PageUp);
        });
        let mut commands = world
            .tui_paged
            .child_commands
            .take()
            .expect("recording child command receiver");
        let handle = world
            .tui_parity_rt
            .as_ref()
            .expect("harness runtime")
            .handle()
            .clone();
        let expected = current.clone();
        let command = handle.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    let line = commands.recv().await.expect("child command socket closed");
                    let value: serde_json::Value =
                        serde_json::from_str(&line).expect("child command is JSON");
                    if value.get("type").and_then(|v| v.as_str()) == Some("get_messages")
                        && value.get("before").and_then(|v| v.as_str()) == Some(expected.as_str())
                    {
                        break value;
                    }
                }
            })
            .await
            .expect("TUI did not route the expected history page request to the child socket")
        });
        world.tui_paged.child_commands = Some(commands);
        let request_id = command
            .get("id")
            .and_then(|v| v.as_str())
            .expect("child history request carries a correlation id")
            .to_string();
        assert!(
            request_id.starts_with("history-page-"),
            "unexpected child history request id: {request_id}"
        );

        let page = page_json(&messages, page_size, Some(&current));
        drive(world, |h| {
            h.route(CHILD, get_messages_response(&request_id, page));
        });
        world.tui_paged.requested_older = true;
        let end = messages.iter().position(|(id, _)| *id == current).unwrap();
        let start = end.saturating_sub(page_size);
        cursor = (start > 0).then(|| messages[start].0.clone());
    }
}

#[when("the operator resumes the session in the TUI")]
fn when_resume(world: &mut TuiWorld) {
    let page = page_json(&world.tui_paged.messages, world.tui_paged.page_size, None);
    drive(world, |h| {
        h.event(get_messages_response("resume-messages", page));
    });
}

#[when("the operator retries scroll back after the page enqueue fails")]
fn when_retry_page_after_enqueue_failure(world: &mut TuiWorld) {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .handle()
        .clone();
    for _ in 0..2 {
        drive(world, |h| {
            h.press(Key::PageUp);
        });
        let handled = {
            let h = &mut world.tui_parity.as_mut().expect("harness").0;
            handle.block_on(h.handle_next_command_send_failure())
        };
        world.tui_paged.observed_send_failures += usize::from(handled);
    }
}

#[when("the operator retries stub recall after the enqueue fails")]
fn when_retry_stub_after_enqueue_failure(world: &mut TuiWorld) {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("runtime")
        .handle()
        .clone();
    for _ in 0..2 {
        drive(world, |h| {
            h.press(Key::PageUp);
        });
        let handled = {
            let h = &mut world.tui_parity.as_mut().expect("harness").0;
            handle.block_on(h.handle_next_command_send_failure())
        };
        world.tui_paged.observed_send_failures += usize::from(handled);
    }
}

#[when("the operator requests the full content for that history message")]
fn when_request_stub_full(world: &mut TuiWorld) {
    let _ = drain(world);
    // Auto-recall on scroll: scrolling the stub into view requests its full body.
    drive(world, |h| {
        h.press(Key::PageUp);
    });
    let cmds = drain(world);
    let (req_id, message_id) = cmds
        .iter()
        .find_map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).ok()?;
            if v.get("type").and_then(|t| t.as_str()) != Some("get_message") {
                return None;
            }
            Some((
                v.get("id")?.as_str()?.to_string(),
                v.get("messageId")?.as_str()?.to_string(),
            ))
        })
        .expect("scrolling a stub into view must auto-request its full content");
    assert_eq!(message_id, world.tui_paged.stub_id.clone().unwrap());
    let full = world.tui_paged.stub_full.clone().unwrap();
    drive(world, |h| {
        h.event(Event::Response {
            id: Some(req_id),
            command: "get_message".into(),
            success: true,
            data: Some(serde_json::json!({
                "id": message_id,
                "role": "assistant",
                "content": full,
            })),
            error: None,
        });
    });
}

// ── Then ────────────────────────────────────────────────────────────────────

#[then("the main chat should show the newest prior messages")]
#[then("the main chat should show the newest resumed messages")]
fn then_shows_newest(world: &mut TuiWorld) {
    let newest = world.tui_paged.messages.last().unwrap().1.clone();
    let frame = active_chat_text(world);
    assert!(
        frame.contains(&newest),
        "newest message '{newest}' must be visible:\n{frame}"
    );
}

#[then("the chat should show whether older history is available")]
fn then_older_availability_known(world: &mut TuiWorld) {
    let _ = drain(world);
    drive(world, |h| {
        h.press(Key::PageUp);
    });
    let cmds = drain(world);
    assert!(
        find_older_request(&cmds).is_some(),
        "the client must know older history is available and fetch it on scroll; commands={cmds:?}"
    );
}

#[then("the chat should reveal the first session message")]
#[then("the sub-agent chat should reveal the first sub-agent message")]
fn then_reveals_first(world: &mut TuiWorld) {
    let first = world.tui_paged.messages.first().unwrap().1.clone();
    let frame = active_chat_text(world);
    assert!(
        frame.contains(&first),
        "the first session message '{first}' must be revealed:\n{frame}"
    );
}

#[then("the revealed history should contain each session message exactly once")]
#[then("the revealed sub-agent history should contain each sub-agent message exactly once")]
#[then("the chat should continue to show every session message")]
fn then_each_once(world: &mut TuiWorld) {
    let messages = world.tui_paged.messages.clone();
    let frame = active_chat_text(world);
    for (_, content) in &messages {
        assert_eq!(
            frame.matches(content.as_str()).count(),
            1,
            "'{content}' must appear exactly once:\n{frame}"
        );
    }
}

#[then("the revealed history should contain no interior gap")]
#[then("the revealed sub-agent history should contain no interior gap")]
fn then_no_gap(world: &mut TuiWorld) {
    let messages = world.tui_paged.messages.clone();
    let frame = active_chat_text(world);
    let mut last = 0usize;
    for (_, content) in &messages {
        let pos = frame
            .find(content.as_str())
            .unwrap_or_else(|| panic!("'{content}' missing (interior gap):\n{frame}"));
        assert!(pos >= last, "'{content}' is out of order (gap):\n{frame}");
        last = pos;
    }
}

#[then("the chat should not request older history")]
fn then_no_older_request(world: &mut TuiWorld) {
    assert!(
        !world.tui_paged.requested_older,
        "a one-slice history must not request older pages"
    );
}

#[then("the chat should request older history")]
fn then_requests_older(world: &mut TuiWorld) {
    assert!(
        world.tui_paged.requested_older,
        "scrolling to the top of a paged history must request older pages"
    );
}

#[then("the oldest session message should become visible")]
fn then_oldest_visible(world: &mut TuiWorld) {
    let oldest = world.tui_paged.messages.first().unwrap().1.clone();
    let frame = active_chat_text(world);
    assert!(
        frame.contains(&oldest),
        "the oldest session message '{oldest}' must become visible:\n{frame}"
    );
}

#[then("older resumed messages should be reachable by scrolling back")]
fn then_older_resumed_reachable(world: &mut TuiWorld) {
    scroll_master_to_beginning(world);
    let oldest = world.tui_paged.messages.first().unwrap().1.clone();
    let frame = active_chat_text(world);
    assert!(
        world.tui_paged.requested_older,
        "resume must keep older history reachable via paging"
    );
    assert!(
        frame.contains(&oldest),
        "older resumed history '{oldest}' must be reachable by scrolling back:\n{frame}"
    );
}

#[then("both older history attempts should reach command failure handling")]
#[then("both stub recall attempts should reach command failure handling")]
fn then_both_retries_reach_failure_handling(world: &mut TuiWorld) {
    assert_eq!(
        world.tui_paged.observed_send_failures, world.tui_paged.expected_send_failures,
        "each retry must enqueue independently and report its own send failure"
    );
}

#[then("the recalled content should replace the stubbed history entry")]
fn then_stub_replaced(world: &mut TuiWorld) {
    let full = world.tui_paged.stub_full.clone().unwrap();
    let frame = active_chat_text(world);
    assert!(
        frame.contains(&full),
        "recalled content must replace the stub:\n{frame}"
    );
    assert!(
        !frame.contains("recall available"),
        "the stub body must be gone after recall:\n{frame}"
    );
}
