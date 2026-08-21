//! #1259 live-inflight buffer characterization tests.
//!
//! Focus/refocus mid-turn retention, mid-turn ledger races, pre-authority
//! connect races, entry cap, and turn-end reconciliation.

use super::*;
use serde_json::json;

fn subagent(id: &str, status: &str) -> crate::protocol::client::SubagentInfoEvent {
    crate::protocol::client::SubagentInfoEvent {
        agent_uuid: None,
        display_name: None,
        agent_id: id.to_string(),
        status: status.to_string(),
        last_tool: None,
        last_error: None,
        compact: false,
        pid: 0,
        socket_path: None,
        parent_id: None,
        workflow: None,
        read_only: false,
        execution_backend: None,
        environment: None,
    }
}

fn feed_with_rx() -> (FeedState, mpsc::Receiver<Command>) {
    let (cmd_tx, cmd_rx) = mpsc::channel(8);
    let handle = tokio::spawn(async {});
    (
        FeedState {
            cmd_tx,
            handle,
            inspection_only: false,
            epoch: 0,
            rev: 0,
            last_fresh_at: None,
            supports_sync: false,
            pending_rev: None,
            transcript: crate::agents::ledger::LedgerTranscript::default(),
            authority: crate::agents::feed::FeedAuthority::WarmSync,
        },
        cmd_rx,
    )
}

/// #1259 (a): focusing a busy child mid-turn must show committed history plus
/// ALL in-flight messages produced before focus — not only the live tail after
/// the focus moment.
#[tokio::test]
async fn focus_mid_turn_backfills_inflight_live_prefix_before_streaming_continues() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed, _rx) = feed_with_rx();
    feed.supports_sync = true;
    app.ac_mut().roster.feeds.insert("worker".into(), feed);
    app.update_subagent_bar(vec![subagent("worker", "running")]);
    // Stay on master while the child streams its in-flight turn.
    app.ensure_session("worker");
    app.route_sync_response(
        "worker",
        &json!({
            "epoch": 1,
            "rev": 1,
            "messages": [{"id":"u1","role":"user","content":"initial task"}],
            "nextRev": null,
            "caughtUp": true,
            "resync": false
        }),
    );

    app.route_subagent_event("worker", Event::TurnStart);
    app.route_subagent_event(
        "worker",
        Event::Token {
            token: "prefix-a".into(),
        },
    );
    app.route_subagent_event(
        "worker",
        Event::Token {
            token: " prefix-b".into(),
        },
    );

    // Focus mid-turn: the full in-flight prefix must already be visible.
    app.select_agent(Some("worker"));
    let entries = app.ac().roster.sessions["worker"].chat.entries();
    assert!(
        matches!(
            entries,
            [
                ChatEntry::User { text, .. },
                            ChatEntry::Assistant { text: live, streaming: true, .. }
            ] if text == "initial task" && live == "prefix-a prefix-b"
        ),
        "focus mid-turn must show committed history plus the full in-flight live prefix: {entries:?}"
    );

    // Streaming continues seamlessly after focus.
    app.route_subagent_event(
        "worker",
        Event::Token {
            token: " suffix".into(),
        },
    );
    let entries = app.ac().roster.sessions["worker"].chat.entries();
    assert!(
        matches!(
            entries,
            [
                ChatEntry::User { text, .. },
                            ChatEntry::Assistant { text: live, streaming: true, .. }
            ] if text == "initial task" && live == "prefix-a prefix-b suffix"
        ),
        "post-focus live tokens must append to the retained in-flight prefix: {entries:?}"
    );
}

/// #1259 (b): focus → unfocus → refocus mid-turn must preserve the transcript
/// (no reset to the task prompt, no lost live content), including when focus
/// refresh re-projects from the committed ledger.
#[tokio::test]
async fn refocus_mid_turn_preserves_inflight_transcript_across_ledger_reproject() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed, mut rx) = feed_with_rx();
    feed.supports_sync = true;
    app.ac_mut().roster.feeds.insert("worker".into(), feed);
    app.update_subagent_bar(vec![subagent("worker", "running")]);
    app.select_agent(Some("worker"));
    app.ensure_session("worker");
    app.route_sync_response(
        "worker",
        &json!({
            "epoch": 1,
            "rev": 1,
            "messages": [{"id":"u1","role":"user","content":"initial task"}],
            "nextRev": null,
            "caughtUp": true,
            "resync": false
        }),
    );
    // Drain any focus-refresh sync that may have been enqueued.
    while rx.try_recv().is_ok() {}

    app.route_subagent_event("worker", Event::TurnStart);
    app.route_subagent_event(
        "worker",
        Event::Token {
            token: "live work".into(),
        },
    );

    app.select_agent(None);
    // Unfocused child keeps streaming; those tokens must not be lost either.
    app.route_subagent_event(
        "worker",
        Event::Token {
            token: " while away".into(),
        },
    );

    app.select_agent(Some("worker"));
    // Simulate the focus-refresh sync that re-projects committed ledger only
    // (no committed assistant yet — the turn is still in flight).
    app.route_sync_response(
        "worker",
        &json!({
            "epoch": 1,
            "rev": 1,
            "messages": [{"id":"u1","role":"user","content":"initial task"}],
            "nextRev": null,
            "caughtUp": true,
            "resync": false
        }),
    );

    let entries = app.ac().roster.sessions["worker"].chat.entries();
    assert!(
        matches!(
            entries,
            [
                ChatEntry::User { text, .. },
                            ChatEntry::Assistant { text: live, streaming: true, .. }
            ] if text == "initial task" && live == "live work while away"
        ),
        "refocus + ledger re-project mid-turn must keep the full in-flight transcript: {entries:?}"
    );

    app.route_subagent_event(
        "worker",
        Event::Token {
            token: " after refocus".into(),
        },
    );
    let entries = app.ac().roster.sessions["worker"].chat.entries();
    assert!(
        matches!(
            entries,
            [
                ChatEntry::User { text, .. },
                            ChatEntry::Assistant { text: live, streaming: true, .. }
            ] if text == "initial task" && live == "live work while away after refocus"
        ),
        "streaming must resume on the retained in-flight transcript after refocus: {entries:?}"
    );
}

/// #1259 review finding 1: a mid-turn higher-rev sync that only extends the
/// committed prefix (no assistant body yet) must NOT wipe the retained live
/// tail that raced ahead of the ledger.
#[tokio::test]
async fn mid_turn_higher_rev_sync_keeps_uncommitted_live_tail() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed, _rx) = feed_with_rx();
    feed.supports_sync = true;
    app.ac_mut().roster.feeds.insert("worker".into(), feed);
    app.update_subagent_bar(vec![subagent("worker", "running")]);
    app.select_agent(Some("worker"));
    app.ensure_session("worker");
    app.route_sync_response(
        "worker",
        &json!({
            "epoch": 1,
            "rev": 1,
            "messages": [{"id":"u1","role":"user","content":"initial task"}],
            "nextRev": null,
            "caughtUp": true,
            "resync": false
        }),
    );

    app.route_subagent_event("worker", Event::TurnStart);
    app.route_subagent_event(
        "worker",
        Event::Token {
            token: "live ahead of ledger".into(),
        },
    );

    // Mid-turn ledger advance: only the user prompt re-committed at a new rev
    // (e.g. turn-start publish racing behind already-streamed tokens).
    app.route_sync_response(
        "worker",
        &json!({
            "epoch": 1,
            "rev": 2,
            "messages": [{"id":"u1","role":"user","content":"initial task"}],
            "nextRev": null,
            "caughtUp": true,
            "resync": false
        }),
    );

    let entries = app.ac().roster.sessions["worker"].chat.entries();
    assert!(
        matches!(
            entries,
            [
                ChatEntry::User { text, .. },
                            ChatEntry::Assistant { text: live, streaming: true, .. }
            ] if text == "initial task" && live == "live ahead of ledger"
        ),
        "mid-turn higher-rev sync without assistant must keep live tail: {entries:?}"
    );
}

/// #1259 review finding 2: live events on a warm-sync feed before the first
/// authoritative sync response must still be retained and reattached on focus.
#[tokio::test]
async fn pre_authority_live_events_retained_across_first_sync() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    // WarmSync, supports_sync not yet latched — the initial connect race.
    let (feed, _rx) = feed_with_rx();
    app.ac_mut().roster.feeds.insert("worker".into(), feed);
    app.update_subagent_bar(vec![subagent("worker", "running")]);
    app.ensure_session("worker");

    app.route_subagent_event("worker", Event::TurnStart);
    app.route_subagent_event(
        "worker",
        Event::Token {
            token: "before first sync".into(),
        },
    );

    // First sync promotes authority and projects committed history only.
    app.route_sync_response(
        "worker",
        &json!({
            "epoch": 1,
            "rev": 1,
            "messages": [{"id":"u1","role":"user","content":"initial task"}],
            "nextRev": null,
            "caughtUp": true,
            "resync": false
        }),
    );

    app.select_agent(Some("worker"));
    let entries = app.ac().roster.sessions["worker"].chat.entries();
    assert!(
        matches!(
            entries,
            [
                ChatEntry::User { text, .. },
                            ChatEntry::Assistant { text: live, streaming: true, .. }
            ] if text == "initial task" && live == "before first sync"
        ),
        "pre-authority live tokens must survive first sync and attach on focus: {entries:?}"
    );
}

/// #1259 review finding 3: live_inflight is entry-capped so a long unfocused
/// stream cannot grow a second unbounded transcript.
#[tokio::test]
async fn live_inflight_buffer_is_entry_capped() {
    use crate::agents::view::LIVE_INFLIGHT_ENTRY_CAP;

    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed, _rx) = feed_with_rx();
    feed.supports_sync = true;
    app.ac_mut().roster.feeds.insert("worker".into(), feed);
    app.update_subagent_bar(vec![subagent("worker", "running")]);
    app.ensure_session("worker");
    app.route_sync_response(
        "worker",
        &json!({
            "epoch": 1,
            "rev": 1,
            "messages": [{"id":"u1","role":"user","content":"initial task"}],
            "nextRev": null,
            "caughtUp": true,
            "resync": false
        }),
    );

    app.route_subagent_event("worker", Event::TurnStart);
    // Force many distinct entries via tool start/end pairs (each pair = 1 entry).
    let overshoot = LIVE_INFLIGHT_ENTRY_CAP + 32;
    for i in 0..overshoot {
        let id = format!("tool-{i}");
        app.route_subagent_event(
            "worker",
            Event::ToolExecutionStart {
                tool_call_id: id.clone(),
                tool_name: "bash".into(),
                args: json!({"command": "echo"}),
            },
        );
        app.route_subagent_event(
            "worker",
            Event::ToolExecutionEnd {
                tool_call_id: id,
                tool_name: "bash".into(),
                result: json!("ok"),
                is_error: false,
            },
        );
    }

    let live_n = app.ac().roster.sessions["worker"]
        .live_inflight
        .entry_count();
    assert!(
        live_n <= LIVE_INFLIGHT_ENTRY_CAP,
        "live_inflight must stay within entry cap ({LIVE_INFLIGHT_ENTRY_CAP}), got {live_n}"
    );
    let has_truncation = app.ac().roster.sessions["worker"]
        .live_inflight
        .entries()
        .iter()
        .any(|e| matches!(e, ChatEntry::Status { text } if text.contains("truncated")));
    assert!(
        has_truncation,
        "overflow must leave a visible truncation marker"
    );
}

/// #1259 review follow-up: an assistant committed in a prior turn must not
/// cause a user-only revision advance in the current turn to erase its live tail.
#[tokio::test]
async fn later_turn_user_rev_advance_keeps_current_live_tail() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed, _rx) = feed_with_rx();
    feed.supports_sync = true;
    app.ac_mut().roster.feeds.insert("worker".into(), feed);
    app.update_subagent_bar(vec![subagent("worker", "running")]);
    app.select_agent(Some("worker"));
    app.ensure_session("worker");
    app.route_sync_response(
        "worker",
        &json!({
            "epoch": 1,
            "rev": 2,
            "messages": [
                {"id":"u1","role":"user","content":"first task"},
                {"id":"a1","role":"assistant","content":"first answer"}
            ],
            "nextRev": null,
            "caughtUp": true,
            "resync": false
        }),
    );

    app.route_subagent_event("worker", Event::TurnStart);
    app.route_subagent_event(
        "worker",
        Event::Token {
            token: "second answer live".into(),
        },
    );
    app.route_sync_response(
        "worker",
        &json!({
            "epoch": 1,
            "rev": 3,
            "messages": [{"id":"u2","role":"user","content":"second task"}],
            "nextRev": null,
            "caughtUp": true,
            "resync": false
        }),
    );

    let entries = app.ac().roster.sessions["worker"].chat.entries();
    assert!(
        matches!(
            entries,
            [
                ChatEntry::User { text: first, .. },
                            ChatEntry::Assistant { text: prior, streaming: false, .. },
                ChatEntry::User { text: second, .. },
                            ChatEntry::Assistant { text: live, streaming: true, .. }
            ] if first == "first task"
                && prior == "first answer"
                && second == "second task"
                && live == "second answer live"
        ),
        "a prior-turn assistant must not supersede the current live tail: {entries:?}"
    );
}

/// #1259 (c): turn-end ledger reconciliation must produce the committed
/// transcript exactly once — no duplicates, no gaps — after a retained
/// in-flight live buffer was shown.
#[tokio::test]
async fn inflight_live_buffer_reconciles_without_duplication_at_turn_end() {
    let mut h = super::tui_harness::TuiHarness::new().await;
    let app = h.app_mut();
    let (mut feed, _rx) = feed_with_rx();
    feed.supports_sync = true;
    app.ac_mut().roster.feeds.insert("worker".into(), feed);
    app.update_subagent_bar(vec![subagent("worker", "running")]);
    app.ensure_session("worker");
    app.route_sync_response(
        "worker",
        &json!({
            "epoch": 1,
            "rev": 1,
            "messages": [{"id":"u1","role":"user","content":"initial task"}],
            "nextRev": null,
            "caughtUp": true,
            "resync": false
        }),
    );

    // Produce the full in-flight turn while unfocused, then focus.
    app.route_subagent_event("worker", Event::TurnStart);
    app.route_subagent_event(
        "worker",
        Event::Token {
            token: "live work".into(),
        },
    );
    app.select_agent(Some("worker"));
    app.route_subagent_event("worker", Event::TurnEnd { message: json!({}) });

    // Committed ledger arrives with the finalized assistant message.
    app.route_sync_response(
        "worker",
        &json!({
            "epoch": 1,
            "rev": 2,
            "messages": [
                {"id":"u1","role":"user","content":"initial task"},
                {"id":"a1","role":"assistant","content":"committed work"}
            ],
            "nextRev": null,
            "caughtUp": true,
            "resync": false
        }),
    );

    let entries = app.ac().roster.sessions["worker"].chat.entries();
    assert!(
        matches!(
            entries,
            [
                ChatEntry::User { text, .. },
                            ChatEntry::Assistant { text: committed, streaming: false, .. }
            ] if text == "initial task" && committed == "committed work"
        ),
        "turn-end ledger reconciliation must replace the in-flight live buffer exactly once: {entries:?}"
    );
}
