//! Region-coverage tests for `app_methods`, `app_rewind`, and `app_response`.
//!
//! These drive the real `App` built by the headless render harness (no TTY,
//! drained socket) and assert on state transitions for the slash-command
//! handlers, selectors, rewind flow, and UDS response dispatch.

use super::app_selection::{SelectionAnchor, TextSelection};
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

#[tokio::test]
async fn update_footer_stats_sets_context_and_clears_zero_cost() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_footer_stats(&serde_json::json!({
        "contextTokens": 42,
        "maxContextTokens": 100,
        "cost": 0.0
    }));
    assert!(a.context_stats_requested);
    let footer = a.footer.render(120).join("\n");
    assert!(footer.contains("42"), "{footer}");
}

#[tokio::test]
async fn update_footer_stats_sets_positive_cost_without_context() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_footer_stats(&serde_json::json!({ "cost": 1.25 }));
    assert!(!a.context_stats_requested);
    let footer = a.footer.render(120).join("\n");
    assert!(footer.contains("$"), "{footer}");
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
async fn replace_chat_with_messages_missing_messages_preserves_chat_and_reports_error() {
    let mut h = harness().await;
    let data = serde_json::json!({});
    let a = h.app_mut();
    a.chat.add_entry(ChatEntry::User {
        text: "keep me".into(),
    });

    a.replace_chat_with_messages(&data);

    let text = chat_text(a);
    assert!(text.contains("keep me"));
    assert!(!text.contains("Session resumed"));
    assert!(text.contains("Invalid resume payload"));
    let notification_text = a
        .notifications
        .render(120)
        .iter()
        .map(|line| super::app_methods::strip_ansi(line))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(notification_text.contains("Invalid resume payload"));
}

#[tokio::test]
async fn replace_chat_with_messages_non_array_messages_preserves_chat_and_reports_error() {
    let mut h = harness().await;
    let data = serde_json::json!({"messages": "bad"});
    let a = h.app_mut();
    a.chat.add_entry(ChatEntry::User {
        text: "keep me".into(),
    });

    a.replace_chat_with_messages(&data);

    let text = chat_text(a);
    assert!(text.contains("keep me"));
    assert!(!text.contains("Session resumed"));
    assert!(text.contains("Invalid resume payload"));
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

// ── app_methods: render hot-path allocation guard (#757) ─────────────
//
// The render path runs on every keystroke, streamed token, and spinner
// tick. `compose_frame` used to deep-clone the entire screen buffer into
// `last_rendered_lines` on EVERY frame, even though that clean copy is only
// ever consumed by mouse text-selection extraction (#528). The clone must be
// guarded behind an active (is/was) selection so idle/streaming frames do not
// allocate a full-screen `Vec<String>` every tick.

#[tokio::test]
async fn compose_frame_skips_full_clone_when_no_selection_active() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.chat.add_entry(ChatEntry::User {
        text: "a line of chat that would be cloned every frame".into(),
    });
    // No selection is or was active.
    assert!(a.selection.is_none());
    let _ = a.compose_frame();
    assert!(
        a.last_rendered_lines.is_empty(),
        "compose_frame must NOT deep-clone the full screen buffer when no \
         text selection is active (#757); last_rendered_lines was populated \
         with {} lines",
        a.last_rendered_lines.len()
    );
}

#[tokio::test]
async fn compose_frame_populates_clone_while_selection_active() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.chat.add_entry(ChatEntry::User {
        text: "selectable text".into(),
    });
    // A drag is in progress: selection is Some.
    a.selection = Some(TextSelection {
        start: SelectionAnchor { col: 0, row: 0 },
        end: SelectionAnchor { col: 5, row: 0 },
    });
    let _ = a.compose_frame();
    assert!(
        !a.last_rendered_lines.is_empty(),
        "compose_frame must keep a clean extraction buffer while a selection \
         is active so mouse-release copy still works (#757/#528)"
    );
}

#[tokio::test]
async fn selection_extraction_works_after_drag_render() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.chat.add_entry(ChatEntry::User {
        text: "hello world".into(),
    });
    // Simulate press+drag keeping the selection live, then a frame renders.
    a.selection = Some(TextSelection {
        start: SelectionAnchor { col: 0, row: 0 },
        end: SelectionAnchor { col: 80, row: 30 },
    });
    let _ = a.compose_frame();
    // The extraction buffer captured during the live drag must let a copy
    // recover visible text even though the optimization skips idle frames.
    let start = SelectionAnchor { col: 0, row: 0 };
    let end = SelectionAnchor { col: 80, row: 30 };
    let text = a.extract_selection(&start, &end);
    assert!(
        text.contains("hello world"),
        "selection extraction should recover rendered text after a drag; got {text:?}"
    );
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
fn format_time_helpers_cover_epoch_leap_and_pre_epoch_paths() {
    assert_eq!(
        super::app_methods::format_utc_minutes(0),
        "1970-01-01 00:00"
    );
    assert_eq!(
        super::app_methods::format_utc_minutes(1_582_934_400),
        "2020-02-29 00:00"
    );
    assert_eq!(super::app_methods::civil_from_days(-719_468), (0, 3, 1));
}

#[test]
fn subagent_activity_line_singular_plural_and_frame_wrap() {
    let one = super::app_methods::subagent_activity_line(1, 0);
    let many = super::app_methods::subagent_activity_line(2, 999);
    assert!(super::app_methods::strip_ansi(&one).contains("1 subagent working"));
    assert!(super::app_methods::strip_ansi(&many).contains("2 subagents working"));
}

#[test]
fn strip_ansi_handles_csi_osc_and_plain() {
    use super::app_methods::strip_ansi;
    assert_eq!(strip_ansi("plain"), "plain");
    assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
    assert_eq!(strip_ansi("\x1b]0;title\x07body"), "body");
    assert_eq!(strip_ansi("\x1b]8;;url\x1b\\link"), "link");
}
