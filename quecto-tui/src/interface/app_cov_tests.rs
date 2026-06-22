//! Region-coverage tests for `app_methods`, `app_rewind`, and `app_response`.
//!
//! These drive the real `App` built by the headless render harness (no TTY,
//! drained socket) and assert on state transitions for the slash-command
//! handlers, selectors, rewind flow, and UDS response dispatch.

use super::tui_harness::TuiHarness;
use super::*;

const MODEL_ID: &str = "anthropic/claude-opus-4-5";

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

fn chat_text(app: &mut App) -> String {
    let lines = app.chat.render(120);
    lines
        .iter()
        .map(|l| super::app_methods::strip_ansi(l))
        .collect::<Vec<_>>()
        .join("\n")
}

// ── app_methods: slash-command handlers ──────────────────────────────

#[tokio::test]
async fn reject_unknown_slash_command_adds_status_and_notifies() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.chat.entry_count();
    a.reject_unknown_slash_command("/bogus");
    assert_eq!(a.chat.entry_count(), before + 1);
    assert!(!a.notifications.is_empty());
    assert!(chat_text(a).contains("/bogus"));
}

#[tokio::test]
async fn show_help_appends_shortcut_status() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.show_help();
    assert!(chat_text(a).contains("Keyboard shortcuts"));
    assert!(chat_text(a).contains("/resume"));
}

#[tokio::test]
async fn show_workflow_status_when_inactive() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.show_workflow_status();
    assert!(chat_text(a).contains("not active"));
}

#[tokio::test]
async fn show_workflow_status_when_active() {
    let mut h = harness().await;
    let wf = serde_json::json!({
        "steps": [
            {"index": 0, "label": "Plan", "phase": "plan", "done": true},
            {"index": 1, "label": "Build it", "phase": "build", "done": false}
        ],
        "progress": {"done": 1, "total": 2},
        "activeIssue": {"number": 7, "title": "thing"}
    });
    h.app_mut().workflow_bar = workflow_bar::parse_workflow_event(&wf);
    let a = h.app_mut();
    a.show_workflow_status();
    let text = chat_text(a);
    assert!(text.contains("Workflow status"), "{text}");
    assert!(text.contains("Build it"), "{text}");
}

#[tokio::test]
async fn show_workflow_status_complete_when_all_steps_done() {
    let mut h = harness().await;
    let wf = serde_json::json!({
        "steps": [{"index": 0, "label": "Plan", "phase": "plan", "done": true}],
        "progress": {"done": 1, "total": 1},
        "activeIssue": {"number": 7, "title": "thing"}
    });
    h.app_mut().workflow_bar = workflow_bar::parse_workflow_event(&wf);
    let a = h.app_mut();
    a.show_workflow_status();
    assert!(chat_text(a).contains("complete"));
}

#[tokio::test]
async fn toggle_workflow_flags_do_not_panic() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.toggle_workflow_auto_continue();
    a.toggle_workflow_completion_nudge();
}

#[tokio::test]
async fn send_session_and_list_commands_do_not_panic() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.send_session_stats();
    a.send_list_sessions();
    a.send_clear_history();
}

#[tokio::test]
async fn send_resume_session_empty_falls_back_to_list() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.send_resume_session("   ");
    a.send_resume_session("my-session");
}

#[tokio::test]
async fn show_session_stats_with_context_updates_footer_flag() {
    let mut h = harness().await;
    let data = serde_json::json!({
        "sessionKey": "cli:foo",
        "totalMessages": 5,
        "tokens": {"input": 10, "output": 20},
        "cost": 0.1234,
        "contextTokens": 100,
        "maxContextTokens": 1000
    });
    let a = h.app_mut();
    a.show_session_stats(&data);
    assert!(a.context_stats_requested);
    assert!(chat_text(a).contains("Session: cli:foo"));
}

#[tokio::test]
async fn show_session_stats_without_context_leaves_flag_false() {
    let mut h = harness().await;
    let data = serde_json::json!({"sessionKey": "cli:bar"});
    let a = h.app_mut();
    a.show_session_stats(&data);
    assert!(!a.context_stats_requested);
}

#[tokio::test]
async fn send_set_model_records_current_model() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.context_stats_requested = true;
    a.send_set_model(MODEL_ID);
    assert_eq!(a.current_model.as_deref(), Some(MODEL_ID));
    assert!(!a.context_stats_requested);
}

// ── app_methods: resume selector ─────────────────────────────────────

#[tokio::test]
async fn open_resume_selector_empty_shows_status_no_selector() {
    let mut h = harness().await;
    let data = serde_json::json!({"sessions": []});
    let a = h.app_mut();
    a.open_resume_selector(&data);
    assert!(a.resume_selector.is_none());
    assert!(chat_text(a).contains("No persisted sessions"));
}

#[tokio::test]
async fn open_resume_selector_with_names_builds_list() {
    let mut h = harness().await;
    let data = serde_json::json!({
        "sessions": [
            {"name": "alpha", "messageCount": 3},
            {"name": "beta"}
        ]
    });
    let a = h.app_mut();
    a.open_resume_selector(&data);
    assert_eq!(a.resume_selector.as_ref().unwrap().item_count(), 2);
}

#[tokio::test]
async fn open_resume_selector_without_names_shows_status() {
    let mut h = harness().await;
    let data = serde_json::json!({"sessions": [{"messageCount": 1}]});
    let a = h.app_mut();
    a.open_resume_selector(&data);
    assert!(a.resume_selector.is_none());
    assert!(chat_text(a).contains("No resumable"));
}

#[tokio::test]
async fn handle_resume_selector_key_enter_selects_and_closes() {
    let mut h = harness().await;
    let data = serde_json::json!({"sessions": [{"name": "alpha", "messageCount": 3}]});
    let a = h.app_mut();
    a.open_resume_selector(&data);
    a.handle_resume_selector_key(&Key::Enter);
    assert!(a.resume_selector.is_none());
}

#[tokio::test]
async fn handle_resume_selector_key_escape_cancels() {
    let mut h = harness().await;
    let data = serde_json::json!({"sessions": [{"name": "alpha"}]});
    let a = h.app_mut();
    a.open_resume_selector(&data);
    a.handle_resume_selector_key(&Key::Escape);
    assert!(a.resume_selector.is_none());
}

#[tokio::test]
async fn handle_resume_selector_key_pending_keeps_selector() {
    let mut h = harness().await;
    let data = serde_json::json!({"sessions": [{"name": "a"}, {"name": "b"}]});
    let a = h.app_mut();
    a.open_resume_selector(&data);
    a.handle_resume_selector_key(&Key::Down);
    assert!(a.resume_selector.is_some());
}

// ── app_methods: replace chat with messages ──────────────────────────

#[tokio::test]
async fn replace_chat_with_messages_renders_roles() {
    let mut h = harness().await;
    let data = serde_json::json!({
        "messages": [
            {"role": "user", "content": "hi"},
            {"role": "assistant", "content": "hello there"},
            {"role": "assistant", "content": ""},
            {"role": "system", "content": "ignored"}
        ]
    });
    let a = h.app_mut();
    a.replace_chat_with_messages(&data);
    let text = chat_text(a);
    assert!(text.contains("hi"));
    assert!(text.contains("hello there"));
    assert!(text.contains("Session resumed"));
}

#[tokio::test]
async fn replace_chat_with_messages_empty_still_adds_status() {
    let mut h = harness().await;
    let data = serde_json::json!({});
    let a = h.app_mut();
    a.replace_chat_with_messages(&data);
    assert!(chat_text(a).contains("Session resumed"));
}

// ── app_methods: model selector ──────────────────────────────────────

#[tokio::test]
async fn open_and_cancel_model_selector() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    // Selector now opens only after the fresh model list arrives (ADR-0002
    // on-consume reload), so simulate the list_models response.
    a.handle_list_models(Some(serde_json::json!({ "models": [] })));
    assert!(a.model_selector.is_some());
    a.handle_model_selector_key(&Key::Escape);
    assert!(a.model_selector.is_none());
}

#[tokio::test]
async fn model_selector_enter_selects_and_sets_model() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({ "models": [] })));
    a.handle_model_selector_key(&Key::Enter);
    assert!(a.model_selector.is_none());
    assert!(a.current_model.is_some());
}

#[tokio::test]
async fn model_selector_pending_keeps_open() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({ "models": [] })));
    a.handle_model_selector_key(&Key::Down);
    assert!(a.model_selector.is_some());
}

// ── app_methods: notifications, reset, selection ─────────────────────

#[tokio::test]
async fn notify_pushes_notification() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.notify("hi", NotifyLevel::Info);
    assert!(!a.notifications.is_empty());
}

#[tokio::test]
async fn reset_session_clears_chat_and_notifies() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.chat.add_entry(ChatEntry::User { text: "x".into() });
    a.context_stats_requested = true;
    a.reset_session("New session");
    assert_eq!(a.chat.entry_count(), 0);
    assert!(!a.context_stats_requested);
    assert!(!a.notifications.is_empty());
}

#[tokio::test]
async fn extract_selection_spans_rows_and_normalizes_order() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.last_rendered_lines = vec![
        "hello world".to_string(),
        "second line".to_string(),
        "third row".to_string(),
    ];
    let start = SelectionAnchor { col: 6, row: 0 };
    let end = SelectionAnchor { col: 6, row: 2 };
    let forward = a.extract_selection(&start, &end);
    assert!(forward.starts_with("world"));
    assert!(forward.contains("second line"));
    // Reversed anchors must yield the same selection.
    let reversed = a.extract_selection(&end, &start);
    assert_eq!(forward, reversed);
}

#[tokio::test]
async fn extract_selection_handles_out_of_range_rows() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.last_rendered_lines = vec!["only".to_string()];
    let start = SelectionAnchor { col: 0, row: 0 };
    let end = SelectionAnchor { col: 50, row: 9 };
    let sel = a.extract_selection(&start, &end);
    assert_eq!(sel, "only");
}

// ── app_methods: compose_frame overlay branches ──────────────────────

#[tokio::test]
async fn compose_frame_with_resume_overlay() {
    let mut h = harness().await;
    let data = serde_json::json!({"sessions": [{"name": "alpha", "messageCount": 1}]});
    h.app_mut().open_resume_selector(&data);
    let frame = h.app_mut().compose_frame();
    assert!(!frame.is_empty());
}

#[tokio::test]
async fn compose_frame_with_model_overlay() {
    let mut h = harness().await;
    h.app_mut().open_model_selector();
    let frame = h.app_mut().compose_frame();
    assert!(!frame.is_empty());
}

#[tokio::test]
async fn compose_frame_with_rewind_overlay() {
    let mut h = harness().await;
    let data = serde_json::json!({
        "messages": [{"role": "user", "content": "first turn"}]
    });
    h.app_mut().open_rewind_selector(&data);
    let frame = h.app_mut().compose_frame();
    assert!(!frame.is_empty());
}

// ── app_methods: free functions ──────────────────────────────────────

#[tokio::test]
async fn compose_bottom_shows_subagent_activity_when_idle_with_active_child() {
    let mut h = harness().await;
    h.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent("w1", "running", None),
    ]));
    // Parent idle (no spinner) but a child is active -> activity line rendered.
    let bottom = h.app_mut().compose_bottom(120);
    let joined = bottom.join("\n");
    assert!(joined.contains("working"), "{joined}");
}

#[test]
fn strip_ansi_handles_csi_osc_and_plain() {
    use super::app_methods::strip_ansi;
    assert_eq!(strip_ansi("plain"), "plain");
    assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
    assert_eq!(strip_ansi("\x1b]0;title\x07body"), "body");
    assert_eq!(strip_ansi("\x1b]8;;url\x1b\\link"), "link");
}

// ── app_rewind ───────────────────────────────────────────────────────

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
    respond(a, None, "clear_history", true, None, None);
    respond(a, None, "totally_unknown_command", true, None, None);
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
