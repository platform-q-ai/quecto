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
    assert!(a.last_idle_escape.is_some());
    assert!(a.pending_rewind_open_id.is_none());
    assert!(!a.notifications.is_empty());
}

#[tokio::test]
async fn idle_escape_double_press_requests_messages() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_idle_escape_for_rewind();
    a.handle_idle_escape_for_rewind();
    assert!(a.last_idle_escape.is_none());
    assert!(a.pending_rewind_open_id.is_some());
}

#[tokio::test]
async fn open_rewind_selector_no_messages_key_notifies() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_rewind_selector(&serde_json::json!({}));
    assert!(a.rewind_selector.is_none());
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
    assert_eq!(a.rewind_selector.as_ref().unwrap().item_count(), 2);
}

#[tokio::test]
async fn open_rewind_selector_no_user_turns_notifies() {
    let mut h = harness().await;
    let data = serde_json::json!({"messages": [{"role": "assistant", "content": "x"}]});
    let a = h.app_mut();
    a.open_rewind_selector(&data);
    assert!(a.rewind_selector.is_none());
    assert!(!a.notifications.is_empty());
}

#[tokio::test]
async fn rewind_selector_enter_requests_rewind() {
    let mut h = harness().await;
    let data = serde_json::json!({"messages": [{"role": "user", "content": "turn"}]});
    let a = h.app_mut();
    a.open_rewind_selector(&data);
    a.handle_rewind_selector_key(&Key::Enter);
    assert!(a.rewind_selector.is_none());
    assert!(a.pending_rewind_apply_id.is_some());
}

#[tokio::test]
async fn rewind_selector_escape_cancels() {
    let mut h = harness().await;
    let data = serde_json::json!({"messages": [{"role": "user", "content": "turn"}]});
    let a = h.app_mut();
    a.open_rewind_selector(&data);
    a.handle_rewind_selector_key(&Key::Escape);
    assert!(a.rewind_selector.is_none());
    assert!(a.pending_rewind_apply_id.is_none());
}

#[tokio::test]
async fn rewind_selector_invalid_value_notifies_error() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.rewind_selector = Some(SelectList::new(
        vec![SelectItem {
            value: "not-a-number".into(),
            label: "bad".into(),
            description: None,
        }],
        10,
    ));
    a.handle_rewind_selector_key(&Key::Enter);
    assert!(a.rewind_selector.is_none());
    assert!(a.pending_rewind_apply_id.is_none());
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
    assert!(a.rewind_selector.is_some());
}

#[tokio::test]
async fn rewind_request_ids_are_monotonically_increasing() {
    let mut h = harness().await;
    let a = h.app_mut();
    let seq_before = a.rewind_request_seq;
    // First double-escape generates an open request.
    a.handle_idle_escape_for_rewind();
    a.handle_idle_escape_for_rewind();
    let open_id = a.pending_rewind_open_id.as_ref().unwrap().clone();
    assert!(open_id.contains("rewind-open-"));
    let seq_after_open = a.rewind_request_seq;
    assert_eq!(seq_after_open, seq_before.wrapping_add(1));
    // Now simulate a rewind apply (open selector, press Enter).
    let data = serde_json::json!({"messages": [{"role": "user", "content": "turn"}]});
    a.pending_rewind_open_id = None; // simulate response clearing it
    a.open_rewind_selector(&data);
    a.handle_rewind_selector_key(&Key::Enter);
    let apply_id = a.pending_rewind_apply_id.as_ref().unwrap().clone();
    assert!(apply_id.contains("rewind-to-"));
    let seq_after_apply = a.rewind_request_seq;
    assert_eq!(seq_after_apply, seq_before.wrapping_add(2));
}

#[tokio::test]
async fn double_escape_outside_window_does_not_open_selector() {
    let mut h = harness().await;
    let a = h.app_mut();
    // First Escape arms.
    a.handle_idle_escape_for_rewind();
    assert!(a.last_idle_escape.is_some());
    // Simulate passage of time beyond the window by clearing last_idle_escape.
    a.last_idle_escape = None;
    // Second Escape should arm again (not open selector).
    a.handle_idle_escape_for_rewind();
    assert!(
        a.last_idle_escape.is_some(),
        "should arm again after window expired"
    );
    assert!(
        a.pending_rewind_open_id.is_none(),
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
    a.pending_rewind_open_id = Some("rid".into());
    let data = serde_json::json!({"messages": [{"role": "user", "content": "turn"}]});
    respond(a, Some("rid"), "get_messages", true, Some(data), None);
    assert!(a.rewind_selector.is_some());
    assert!(a.pending_rewind_open_id.is_none());
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
    a.pending_rewind_apply_id = Some("rt".into());
    respond(a, Some("rt"), "rewind_to", true, None, None);
    assert!(a.pending_rewind_apply_id.is_none());
    assert!(!a.notifications.is_empty());
}

#[tokio::test]
async fn response_rewind_to_failure_clears_pending_and_notifies_error() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.pending_rewind_apply_id = Some("rt".into());
    respond(a, Some("rt"), "rewind_to", false, None, Some("bad"));
    assert!(a.pending_rewind_apply_id.is_none());
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
async fn response_clear_history_and_unknown_are_noop() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.master_session.chat.entry_count();
    respond(a, None, "clear_history", true, None, None);
    respond(a, None, "totally_unknown_command", true, None, None);
    // Neither response is surfaced: no chat entries added, no notifications.
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
async fn response_get_subagents_updates_bar() {
    let mut h = harness().await;
    let a = h.app_mut();
    let data = serde_json::json!({
        "subagents": [
            {"agentId": "w1", "status": "running", "pid": 0}
        ]
    });
    respond(a, None, "get_subagents", true, Some(data), None);
    assert!(!a.subagent_local.is_empty());
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
