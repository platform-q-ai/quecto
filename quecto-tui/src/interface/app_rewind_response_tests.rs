//! Region-coverage tests for `app_rewind` and `app_response`, split from
//! `app_cov_tests.rs` to stay within the line-count gate.

use super::tui_harness::TuiHarness;
use super::*;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

fn chat_text(app: &mut App) -> String {
    let lines = app.master_session.chat.render(120);
    lines
        .iter()
        .map(|l| super::app_methods::strip_ansi(l))
        .collect::<Vec<_>>()
        .join("\n")
}

// ── app_rewind ───────────────────────────────────────────────────────

/// No-regression guard (#865): the TUI's OWN internal `get_state`/stats polling
/// flows through `handle_response` (Response events with ids like `init`/
/// `stats-footer`), NOT the tool path, so it must never add a chat entry / box.
#[tokio::test]
async fn internal_state_polling_response_renders_no_box() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.master_session.chat.entry_count();
    a.handle_event(Event::Response {
        id: Some("init".into()),
        command: "get_state".into(),
        success: true,
        data: Some(serde_json::json!({
            "model": "m",
            "sessionKey": "cli:default",
            "workflow": {"mode":"complete","progress":{"done":5,"total":5,"percent":100}}
        })),
        error: None,
    });
    a.handle_event(Event::Response {
        id: Some("stats-footer".into()),
        command: "get_session_stats".into(),
        success: true,
        data: Some(serde_json::json!({"cost": 0.0})),
        error: None,
    });
    assert_eq!(
        a.master_session.chat.entry_count(),
        before,
        "internal get_state/stats polling must not render a chat box (#865)"
    );
}

#[tokio::test]
async fn rewind_preview_truncates_and_sanitizes() {
    let preview = super::app_rewind::rewind_preview("safe \u{1b}[31mvalue");
    assert!(!preview.contains('\u{1b}'));
    assert!(preview.contains("safe"));
}

#[tokio::test]
async fn idle_escape_first_press_arms_notification() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_idle_escape_for_rewind();
    assert!(a.rewind.last_idle_escape.is_some());
    assert!(a.rewind.pending_open_id.is_none());
    assert!(!a.notifications.is_empty());
}

#[tokio::test]
async fn idle_escape_double_press_requests_messages() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_idle_escape_for_rewind();
    a.handle_idle_escape_for_rewind();
    assert!(a.rewind.last_idle_escape.is_none());
    assert!(a.rewind.pending_open_id.is_some());
}

#[tokio::test]
async fn open_rewind_selector_no_messages_key_notifies() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_rewind_selector(&serde_json::json!({}));
    assert!(a.rewind.selector.is_none());
    assert!(!a.notifications.is_empty());
}

#[tokio::test]
async fn open_rewind_selector_builds_turns_in_reverse() {
    let mut h = harness().await;
    let data = serde_json::json!({
        "messages": [
            {"role": "user", "content": "one"},
            {"role": "assistant", "content": "ans"},
            {"role": "user", "content": "two"}
        ]
    });
    let a = h.app_mut();
    a.open_rewind_selector(&data);
    assert_eq!(a.rewind.selector.as_ref().unwrap().item_count(), 2);
}

#[tokio::test]
async fn open_rewind_selector_no_user_turns_notifies() {
    let mut h = harness().await;
    let data = serde_json::json!({"messages": [{"role": "assistant", "content": "x"}]});
    let a = h.app_mut();
    a.open_rewind_selector(&data);
    assert!(a.rewind.selector.is_none());
    assert!(!a.notifications.is_empty());
}

#[tokio::test]
async fn rewind_selector_enter_requests_rewind() {
    let mut h = harness().await;
    let data = serde_json::json!({"messages": [{"role": "user", "content": "turn"}]});
    let a = h.app_mut();
    a.open_rewind_selector(&data);
    a.handle_rewind_selector_key(&Key::Enter);
    assert!(a.rewind.selector.is_none());
    assert!(a.rewind.pending_apply_id.is_some());
}

#[tokio::test]
async fn rewind_selector_escape_cancels() {
    let mut h = harness().await;
    let data = serde_json::json!({"messages": [{"role": "user", "content": "turn"}]});
    let a = h.app_mut();
    a.open_rewind_selector(&data);
    a.handle_rewind_selector_key(&Key::Escape);
    assert!(a.rewind.selector.is_none());
    assert!(a.rewind.pending_apply_id.is_none());
}

#[tokio::test]
async fn rewind_selector_invalid_value_notifies_error() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.rewind.selector = Some(SelectList::new(
        vec![SelectItem {
            value: "not-a-number".into(),
            label: "bad".into(),
            description: None,
        }],
        10,
    ));
    a.handle_rewind_selector_key(&Key::Enter);
    assert!(a.rewind.selector.is_none());
    assert!(a.rewind.pending_apply_id.is_none());
    assert!(!a.notifications.is_empty());
}

#[tokio::test]
async fn rewind_selector_pending_keeps_open() {
    let mut h = harness().await;
    let data = serde_json::json!({
        "messages": [
            {"role": "user", "content": "a"},
            {"role": "user", "content": "b"}
        ]
    });
    let a = h.app_mut();
    a.open_rewind_selector(&data);
    a.handle_rewind_selector_key(&Key::Down);
    assert!(a.rewind.selector.is_some());
}

#[tokio::test]
async fn rewind_request_ids_are_monotonically_increasing() {
    let mut h = harness().await;
    let a = h.app_mut();
    let seq_before = a.rewind.request_seq;
    // First double-escape generates an open request.
    a.handle_idle_escape_for_rewind();
    a.handle_idle_escape_for_rewind();
    let open_id = a.rewind.pending_open_id.as_ref().unwrap().clone();
    assert!(open_id.contains("rewind-open-"));
    let seq_after_open = a.rewind.request_seq;
    assert_eq!(seq_after_open, seq_before.wrapping_add(1));
    // Now simulate a rewind apply (open selector, press Enter).
    let data = serde_json::json!({"messages": [{"role": "user", "content": "turn"}]});
    a.rewind.pending_open_id = None; // simulate response clearing it
    a.open_rewind_selector(&data);
    a.handle_rewind_selector_key(&Key::Enter);
    let apply_id = a.rewind.pending_apply_id.as_ref().unwrap().clone();
    assert!(apply_id.contains("rewind-to-"));
    let seq_after_apply = a.rewind.request_seq;
    assert_eq!(seq_after_apply, seq_before.wrapping_add(2));
}

#[tokio::test]
async fn double_escape_outside_window_does_not_open_selector() {
    let mut h = harness().await;
    let a = h.app_mut();
    // First Escape arms.
    a.handle_idle_escape_for_rewind();
    assert!(a.rewind.last_idle_escape.is_some());
    // Simulate passage of time beyond the window by clearing last_idle_escape.
    a.rewind.last_idle_escape = None;
    // Second Escape should arm again (not open selector).
    a.handle_idle_escape_for_rewind();
    assert!(
        a.rewind.last_idle_escape.is_some(),
        "should arm again after window expired"
    );
    assert!(
        a.rewind.pending_open_id.is_none(),
        "should not open selector"
    );
}

// ── app_response: handle_response dispatch ───────────────────────────

fn respond(
    app: &mut App,
    id: Option<&str>,
    command: &str,
    success: bool,
    data: Option<serde_json::Value>,
    error: Option<&str>,
) {
    app.handle_response(
        id.map(String::from),
        command.to_string(),
        success,
        data,
        error.map(String::from),
    );
}

#[tokio::test]
async fn response_get_state_populates_model_and_agent_id() {
    let mut h = harness().await;
    let data = serde_json::json!({
        "model": "anthropic/claude-opus-4-5",
        "maxContextTokens": 200000,
        "sessionKey": "cli:worker",
        "workflow": {
            "progress": {"done": 1, "total": 3},
            "activeIssue": {"number": 4, "title": "t"},
            "automation": {"autoContinue": true, "completionNudge": true}
        }
    });
    let a = h.app_mut();
    respond(a, None, "get_state", true, Some(data), None);
    assert_eq!(
        a.current_model.as_deref(),
        Some("anthropic/claude-opus-4-5")
    );
    assert_eq!(a.connected_agent_id.as_deref(), Some("worker"));
    assert!(a.workflow_auto_continue);
    assert!(a.workflow_completion_nudge);
    assert!(a.context_stats_requested);
}

#[tokio::test]
async fn response_get_state_default_session_clears_agent_id() {
    let mut h = harness().await;
    let data = serde_json::json!({"sessionKey": "cli:default"});
    let a = h.app_mut();
    respond(a, None, "get_state", true, Some(data), None);
    assert!(a.connected_agent_id.is_none());
}

#[tokio::test]
async fn response_get_state_no_data_is_noop() {
    let mut h = harness().await;
    let a = h.app_mut();
    respond(a, None, "get_state", true, None, None);
    assert!(a.current_model.is_none());
}

#[tokio::test]
async fn response_set_model_success_and_failure() {
    let mut h = harness().await;
    let a = h.app_mut();
    respond(a, None, "set_model", true, None, None);
    assert!(!a.notifications.is_empty());
    respond(a, None, "set_model", false, None, Some("boom"));
}

#[tokio::test]
async fn response_set_workflow_automation_success_and_failure() {
    let mut h = harness().await;
    let a = h.app_mut();
    let data = serde_json::json!({"autoContinue": true, "completionNudge": false});
    respond(a, None, "set_workflow_automation", true, Some(data), None);
    assert!(a.workflow_auto_continue);
    assert!(!a.notifications.is_empty());
    respond(
        a,
        None,
        "set_workflow_automation",
        false,
        None,
        Some("nope"),
    );
}

#[tokio::test]
async fn response_get_session_stats_renders() {
    let mut h = harness().await;
    let a = h.app_mut();
    let data = serde_json::json!({"sessionKey": "cli:s", "totalMessages": 1});
    respond(a, None, "get_session_stats", true, Some(data), None);
    assert!(chat_text(a).contains("Session: cli:s"));
}

#[tokio::test]
async fn response_list_sessions_success_and_failure() {
    let mut h = harness().await;
    let a = h.app_mut();
    let data = serde_json::json!({"sessions": [{"name": "alpha"}]});
    respond(a, None, "list_sessions", true, Some(data), None);
    assert!(a.resume_selector.is_some());
    respond(a, None, "list_sessions", false, None, Some("err"));
    assert!(!a.notifications.is_empty());
}

#[tokio::test]
async fn response_resume_session_success_and_failure() {
    let mut h = harness().await;
    let a = h.app_mut();
    let data = serde_json::json!({"session": "alpha"});
    respond(a, None, "resume_session", true, Some(data), None);
    assert!(!a.notifications.is_empty());
    respond(a, None, "resume_session", false, None, Some("err"));
}

#[tokio::test]
async fn response_get_messages_opens_rewind_when_id_matches() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.rewind.pending_open_id = Some("rid".into());
    let data = serde_json::json!({"messages": [{"role": "user", "content": "turn"}]});
    respond(a, Some("rid"), "get_messages", true, Some(data), None);
    assert!(a.rewind.selector.is_some());
    assert!(a.rewind.pending_open_id.is_none());
}

#[tokio::test]
async fn response_get_messages_replaces_chat_when_no_match() {
    let mut h = harness().await;
    let a = h.app_mut();
    let data = serde_json::json!({"messages": [{"role": "user", "content": "hi"}]});
    respond(a, Some("other"), "get_messages", true, Some(data), None);
    assert!(chat_text(a).contains("Session resumed"));
}

#[tokio::test]
async fn response_rewind_to_success_clears_pending_and_notifies() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.rewind.pending_apply_id = Some("rt".into());
    respond(a, Some("rt"), "rewind_to", true, None, None);
    assert!(a.rewind.pending_apply_id.is_none());
    assert!(!a.notifications.is_empty());
}

#[tokio::test]
async fn response_rewind_to_failure_clears_pending_and_notifies_error() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.rewind.pending_apply_id = Some("rt".into());
    respond(a, Some("rt"), "rewind_to", false, None, Some("bad"));
    assert!(a.rewind.pending_apply_id.is_none());
    assert!(!a.notifications.is_empty());
}

#[tokio::test]
async fn response_rewind_to_unmatched_is_noop() {
    let mut h = harness().await;
    let a = h.app_mut();
    respond(a, Some("zzz"), "rewind_to", true, None, None);
    assert!(a.notifications.is_empty());
}

#[tokio::test]
async fn response_unknown_is_noop() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.master_session.chat.entry_count();
    respond(a, None, "totally_unknown_command", true, None, None);
    // An unknown response is not surfaced: no chat entries, no notifications.
    assert_eq!(
        a.master_session.chat.entry_count(),
        before,
        "noop responses must not add chat entries"
    );
    assert!(
        a.notifications.is_empty(),
        "noop responses must not raise notifications"
    );
}

#[tokio::test]
async fn response_clear_history_signals_workflow_retained() {
    // #897 AC1: clearing history must visibly distinguish the cleared
    // conversation from the workflow engine state, which is retained by design.
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.master_session.chat.entry_count();
    respond(a, None, "clear_history", true, None, None);
    assert_eq!(
        a.master_session.chat.entry_count(),
        before,
        "clear_history must not add chat entries"
    );
    let text = a
        .notifications
        .render(120)
        .iter()
        .map(|line| super::app_methods::strip_ansi(line))
        .collect::<Vec<_>>()
        .join("\n")
        .to_lowercase();
    assert!(
        text.contains("history cleared"),
        "must signal that history was cleared: {text}"
    );
    assert!(
        text.contains("workflow retained"),
        "must signal that the workflow was retained: {text}"
    );
}

#[tokio::test]
async fn response_get_subagents_updates_bar() {
    let mut h = harness().await;
    let a = h.app_mut();
    let data = serde_json::json!({
        "subagents": [
            {"agentId": "w1", "status": "running", "pid": 0}
        ]
    });
    respond(a, None, "get_subagents", true, Some(data), None);
    assert!(!a.subagents.tracked.is_empty());
}

#[tokio::test]
async fn response_agent_error_appends_status_and_resets() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.spinner = Some(super::Spinner::new("x"));
    respond(a, None, "agent_error", false, None, Some("kaboom"));
    assert!(chat_text(a).contains("kaboom"));
    assert!(a.spinner.is_none());
}

#[tokio::test]
async fn response_agent_error_without_message_uses_default() {
    let mut h = harness().await;
    let a = h.app_mut();
    respond(a, None, "agent_error", false, None, None);
    assert!(chat_text(a).contains("unknown error"));
}

// ── #1050: master attach-backfill on --socket connect ────────────────

use super::app_response::ATTACH_BACKFILL_ID;

/// Build a successful `get_messages` payload for the attach-backfill request id.
fn attach_backfill_data(pairs: &[(&str, &str)]) -> serde_json::Value {
    let messages: Vec<serde_json::Value> = pairs
        .iter()
        .flat_map(|(u, a)| {
            [
                serde_json::json!({ "role": "user", "content": u }),
                serde_json::json!({ "role": "assistant", "content": a }),
            ]
        })
        .collect();
    serde_json::json!({ "messages": messages })
}

fn is_attach_backfill_get_messages(line: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
        return false;
    };
    v.get("type").and_then(|t| t.as_str()) == Some("get_messages")
        && v.get("id").and_then(|i| i.as_str()) == Some(ATTACH_BACKFILL_ID)
}

#[tokio::test]
async fn request_master_attach_backfill_sends_get_messages_with_dedicated_id() {
    // On --socket attach the master must request durable history with a
    // dedicated request id so the response path can reconcile (not resume).
    let mut h = harness().await;
    h.app_mut().request_master_attach_backfill();
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter()
            .any(|line| is_attach_backfill_get_messages(line)),
        "attach must request get_messages with id {ATTACH_BACKFILL_ID}, got: {cmds:?}"
    );
}

#[tokio::test]
async fn run_startup_requests_master_attach_backfill() {
    // `App::run` startup must request durable master history so `--socket`
    // attach shows prior session content without waiting for new events.
    let mut h = harness().await;
    let app = h.app_mut();
    app.should_exit = true;
    assert_eq!(app.run().await, 0);
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter()
            .any(|line| is_attach_backfill_get_messages(line)),
        "run() startup must send get_messages id={ATTACH_BACKFILL_ID}, got: {cmds:?}"
    );
}

#[tokio::test]
async fn attach_backfill_prepends_history_and_preserves_live_tokens() {
    // Live tokens can race ahead of the attach get_messages response. The
    // backfill must PREPEND history above live content — never wholesale
    // replace that drops the live stream (#1050, parity with #828).
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_event(Event::AgentStart);
    a.handle_event(Event::Token {
        token: "LIVE_AFTER_ATTACH".into(),
    });
    respond(
        a,
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        Some(attach_backfill_data(&[(
            "earlier question",
            "earlier answer",
        )])),
        None,
    );

    let frame = chat_text(a);
    assert!(
        frame.contains("earlier question") && frame.contains("earlier answer"),
        "attach backfill must render prior history:\n{frame}"
    );
    assert!(
        frame.contains("LIVE_AFTER_ATTACH"),
        "late attach backfill must NOT drop live tokens:\n{frame}"
    );
    let hist = frame.find("earlier answer").expect("history present");
    let live = frame.find("LIVE_AFTER_ATTACH").expect("live present");
    assert!(
        hist < live,
        "history must be PREPENDED above live content:\n{frame}"
    );
    assert!(
        !frame.contains("Session resumed"),
        "attach backfill must not use the resume replace path:\n{frame}"
    );
}

#[tokio::test]
async fn attach_backfill_is_idempotent_and_does_not_duplicate_history() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_event(Event::Token {
        token: "LIVEONE".into(),
    });
    let data = attach_backfill_data(&[("the question", "the answer")]);
    respond(
        a,
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        Some(data.clone()),
        None,
    );
    respond(
        a,
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        Some(data),
        None,
    );

    let frame = chat_text(a);
    assert_eq!(
        frame.matches("the answer").count(),
        1,
        "re-delivered attach backfill must not duplicate history:\n{frame}"
    );
    assert!(
        frame.contains("LIVEONE"),
        "re-delivered attach backfill must not drop live content:\n{frame}"
    );
}

#[tokio::test]
async fn empty_attach_backfill_does_not_latch_guard_against_later_history() {
    let mut h = harness().await;
    let a = h.app_mut();
    respond(
        a,
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        Some(attach_backfill_data(&[])),
        None,
    );
    respond(
        a,
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        Some(attach_backfill_data(&[("real question", "real answer")])),
        None,
    );

    let frame = chat_text(a);
    assert!(
        frame.contains("real question") && frame.contains("real answer"),
        "an empty attach backfill must not suppress a later populated history:\n{frame}"
    );
}

#[tokio::test]
async fn attach_backfill_into_idle_master_renders_full_history_in_order() {
    let mut h = harness().await;
    let a = h.app_mut();
    respond(
        a,
        Some(ATTACH_BACKFILL_ID),
        "get_messages",
        true,
        Some(attach_backfill_data(&[("first question", "first answer")])),
        None,
    );

    let frame = chat_text(a);
    let q = frame
        .find("first question")
        .expect("history question present");
    let apos = frame.find("first answer").expect("history answer present");
    assert!(
        q < apos,
        "idle attach backfill must render history in order:\n{frame}"
    );
    assert!(
        !frame.contains("Session resumed"),
        "attach backfill must reconcile, not the resume replace path:\n{frame}"
    );
}

#[tokio::test]
async fn resume_get_messages_still_replaces_chat_when_not_attach_backfill() {
    // Non-attach get_messages (resume / rewind-refresh) must keep the wholesale
    // replace path and "Session resumed" status (#1050 must not break #resume).
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_event(Event::Token {
        token: "SHOULD_BE_CLEARED".into(),
    });
    respond(
        a,
        Some("resume-messages"),
        "get_messages",
        true,
        Some(attach_backfill_data(&[(
            "resumed user",
            "resumed assistant",
        )])),
        None,
    );
    let frame = chat_text(a);
    assert!(
        frame.contains("Session resumed"),
        "resume path must still replace chat:\n{frame}"
    );
    assert!(
        frame.contains("resumed user"),
        "resume path must show resumed messages:\n{frame}"
    );
    assert!(
        !frame.contains("SHOULD_BE_CLEARED"),
        "resume replace must clear prior live content:\n{frame}"
    );
}

#[tokio::test]
async fn rewind_open_get_messages_still_opens_selector_over_attach_path() {
    // A rewind-pending get_messages id must open the rewind selector, never
    // attach-backfill reconcile, even if history is also pending.
    let mut h = harness().await;
    let a = h.app_mut();
    a.rewind.pending_open_id = Some("rewind-open-1".into());
    respond(
        a,
        Some("rewind-open-1"),
        "get_messages",
        true,
        Some(attach_backfill_data(&[("turn one", "reply one")])),
        None,
    );
    assert!(
        a.rewind.selector.is_some(),
        "rewind pending id must open the rewind selector"
    );
    assert!(
        a.rewind.pending_open_id.is_none(),
        "rewind open id must be cleared after handling"
    );
    let frame = chat_text(a);
    assert!(
        !frame.contains("turn one"),
        "rewind open must not inject history into chat:\n{frame}"
    );
}
