//! Step definitions for `tui_paged_history.feature` (#1061 / ADR-0008 part 3).
//!
//! These drive the REAL TUI backfill/scroll/recall paths through the headless
//! render harness (`quecto_tui::interface::app::tui_harness`) — the same App the
//! binary runs. Pages are delivered as the wire `get_messages`/`get_message`
//! responses the server would send, and assertions read observable state: the
//! rendered transcript and the commands the client emits.

use super::*;
use quecto_tui::infrastructure::client::Event;
use quecto_tui::interface::app::tui_harness::TuiHarness;
use quecto_tui::interface::keys::Key;

/// Per-scenario paged-history fixture stored on the World.
#[derive(Debug, Default)]
pub struct PagedHistoryState {
    /// Full chronological (id, content) history the server would page over.
    pub(super) messages: Vec<(String, String)>,
    pub(super) page_size: usize,
    /// Whether a scroll-back step observed an older-history request.
    pub(super) requested_older: bool,
    /// Commands decoded by the active child's real Unix socket.
    pub(super) stub_id: Option<String>,
    pub(super) stub_full: Option<String>,
    pub(super) expected_send_failures: usize,
    pub(super) observed_send_failures: usize,
    pub(super) stale_page_request_id: Option<String>,
    pub(super) stale_page_cursor: Option<String>,
}

// ── harness plumbing ────────────────────────────────────────────────────────

pub(super) fn init_harness(world: &mut TuiWorld) {
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let h = rt.block_on(TuiHarness::new());
    world.tui_parity_rt = Some(rt);
    world.tui_parity = Some(TuiParityHarness(h));
}

/// Run a closure against the harness inside the runtime context (key/select
/// paths spawn background tasks, so they need a live runtime).
pub(super) fn drive<R>(world: &mut TuiWorld, f: impl FnOnce(&mut TuiHarness) -> R) -> R {
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
pub(super) fn drain(world: &mut TuiWorld) -> Vec<String> {
    let handle = world
        .tui_parity_rt
        .as_ref()
        .expect("harness runtime")
        .handle()
        .clone();
    let h = &mut world.tui_parity.as_mut().expect("harness").0;
    handle.block_on(h.drain_commands())
}

pub(super) fn active_chat_text(world: &mut TuiWorld) -> String {
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

pub(super) fn get_messages_response(id: &str, data: serde_json::Value) -> Event {
    Event::Response {
        id: Some(id.into()),
        command: "get_messages".into(),
        success: true,
        data: Some(data),
        error: None,
    }
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

#[given("the TUI has an older history page in flight")]
fn given_older_page_in_flight(world: &mut TuiWorld) {
    given_attached_enough_history(world);
    let _ = drain(world);
    drive(world, |h| {
        h.press(Key::PageUp);
    });
    let commands = drain(world);
    let (request_id, cursor) = find_older_request(&commands)
        .expect("scrolling paged history should issue an older-page request");
    world.tui_paged.stale_page_request_id = Some(request_id);
    world.tui_paged.stale_page_cursor = Some(cursor);
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

#[when("the operator resumes the session in the TUI")]
fn when_resume(world: &mut TuiWorld) {
    // Drive the REAL resume path: acknowledge a resume_session so the app
    // itself emits the transcript request, then answer that request using the
    // id the app chose. Hard-coding the id here would leave resume request
    // emission and correlation unexercised.
    drive(world, |h| {
        h.event(Event::Response {
            id: Some("resume".into()),
            command: "resume_session".into(),
            success: true,
            data: Some(serde_json::json!({ "session": "resumed-session" })),
            error: None,
        });
    });
    let request_id = drain(world)
        .iter()
        .find_map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).ok()?;
            (value.get("type").and_then(|v| v.as_str()) == Some("get_messages"))
                .then(|| value.get("id")?.as_str().map(str::to_owned))?
        })
        .expect("resume must request the restored transcript");
    let page = page_json(&world.tui_paged.messages, world.tui_paged.page_size, None);
    drive(world, |h| {
        h.event(get_messages_response(&request_id, page));
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

#[when("the operator starts a new conversation before that page arrives")]
fn when_new_conversation_before_page_arrives(world: &mut TuiWorld) {
    drive(world, |h| h.reset_master_session());
    let _ = drain(world);
    let request_id = world
        .tui_paged
        .stale_page_request_id
        .clone()
        .expect("stale page request id");
    let cursor = world
        .tui_paged
        .stale_page_cursor
        .clone()
        .expect("stale page cursor");
    deliver_master_page(world, &request_id, Some(&cursor));
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
    if message_id == "oversized-stub" {
        let page_bytes = quecto_line_io::PROTOCOL_LINE_CAP_BYTES / 4;
        let mut offset = 0usize;
        let mut request_id = req_id;
        let mut pages = 0usize;
        while offset < full.len() {
            let end = offset.saturating_add(page_bytes).min(full.len());
            let data = serde_json::json!({
                "id": message_id,
                "role": "assistant",
                "content": &full[offset..end],
                "offset": offset,
                "nextOffset": end,
                "contentLength": full.len(),
                "hasMoreContent": end < full.len(),
            });
            let encoded = serde_json::json!({
                "type": "response",
                "id": request_id.clone(),
                "command": "get_message",
                "success": true,
                "data": data.clone(),
            })
            .to_string();
            assert!(
                encoded.len() < quecto_line_io::PROTOCOL_LINE_CAP_BYTES,
                "synthetic TUI page must stay below the protocol cap"
            );
            drive(world, |h| {
                h.event(Event::Response {
                    id: Some(request_id),
                    command: "get_message".into(),
                    success: true,
                    data: Some(data),
                    error: None,
                });
            });
            pages += 1;
            offset = end;
            if offset == full.len() {
                break;
            }
            let cmds = drain(world);
            let next = cmds
                .iter()
                .find_map(|line| {
                    let value: serde_json::Value = serde_json::from_str(line).ok()?;
                    (value.get("type").and_then(|v| v.as_str()) == Some("get_message")
                        && value.get("offset").and_then(|v| v.as_u64()) == Some(offset as u64))
                    .then_some(value)
                })
                .expect("partial oversized recall should request the next bounded page");
            assert_eq!(
                next.get("limit").and_then(|v| v.as_u64()),
                Some(page_bytes as u64),
                "every follow-up must retain the bounded page size"
            );
            request_id = next["id"].as_str().unwrap().to_string();
        }
        assert!(pages >= 3, "oversized fixture must exercise multiple pages");
        return;
    }
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
fn then_reveals_first(world: &mut TuiWorld) {
    let first = world.tui_paged.messages.first().unwrap().1.clone();
    let frame = active_chat_text(world);
    assert!(
        frame.contains(&first),
        "the first session message '{first}' must be revealed:\n{frame}"
    );
}

#[then("the revealed history should contain each session message exactly once")]
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

#[then("the late older page should not appear in the new conversation")]
fn then_late_page_absent(world: &mut TuiWorld) {
    let frame = active_chat_text(world);
    assert!(
        !frame.contains("message 0") && !frame.contains("message 1"),
        "late history from the old conversation must be ignored:\n{frame}"
    );
}

#[then("scrolling the new conversation should not request the old history cursor")]
fn then_old_cursor_not_requested(world: &mut TuiWorld) {
    let _ = drain(world);
    drive(world, |h| {
        h.press(Key::PageUp);
    });
    let commands = drain(world);
    let old_cursor = world
        .tui_paged
        .stale_page_cursor
        .as_deref()
        .expect("stale cursor");
    assert!(
        find_older_request(&commands).is_none_or(|(_, cursor)| cursor != old_cursor),
        "new conversation must not page with old cursor {old_cursor}; commands={commands:?}"
    );
}

#[then("the recalled content should replace the stubbed history entry")]
#[then("the recalled content should replace the history entry with the complete oversized body")]
fn then_stub_replaced(world: &mut TuiWorld) {
    let full = world.tui_paged.stub_full.clone().unwrap();
    if full.len() > quecto_line_io::PROTOCOL_LINE_CAP_BYTES {
        // Observe the body the APP reassembled, not a test-local fixture: the
        // rendered frame is width-wrapped, so an oversized body cannot be
        // substring-matched against it.
        let texts = drive(world, |h| h.master_assistant_texts());
        assert!(
            texts.iter().any(|text| text == &full),
            "all oversized pages must be reassembled exactly into the transcript; \
             got {} assistant entries with lengths {:?} (expected one of length {})",
            texts.len(),
            texts.iter().map(String::len).collect::<Vec<_>>(),
            full.len()
        );
    } else {
        let frame = active_chat_text(world);
        assert!(
            frame.contains(&full),
            "recalled content must replace the stub:\n{frame}"
        );
    }
    let frame = active_chat_text(world);
    assert!(
        !frame.contains("recall available"),
        "the stub body must be gone after recall:\n{frame}"
    );
}
