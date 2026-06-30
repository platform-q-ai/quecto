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
    let lines = app.master_session.chat.render(120);
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
    let before = a.master_session.chat.entry_count();
    a.reject_unknown_slash_command("/bogus");
    assert_eq!(a.master_session.chat.entry_count(), before + 1);
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
    h.app_mut().master_session.workflow_bar = workflow_bar::parse_workflow_event(&wf);
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
    h.app_mut().master_session.workflow_bar = workflow_bar::parse_workflow_event(&wf);
    let a = h.app_mut();
    a.show_workflow_status();
    assert!(chat_text(a).contains("complete"));
}

#[tokio::test]
async fn toggle_workflow_flags_send_automation_commands() {
    let mut h = harness().await;
    h.app_mut().toggle_workflow_auto_continue();
    h.app_mut().toggle_workflow_completion_nudge();
    let cmds = h.drain_commands().await;
    // Each toggle sends a distinct automation flag, flipped on from the default off.
    assert!(
        cmds.iter()
            .any(|c| c.contains("\"type\":\"set_workflow_automation\"")
                && c.contains("\"autoContinue\":true")),
        "auto-continue toggle should set autoContinue:true: {cmds:?}"
    );
    assert!(
        cmds.iter()
            .any(|c| c.contains("\"type\":\"set_workflow_automation\"")
                && c.contains("\"completionNudge\":true")),
        "completion-nudge toggle should set completionNudge:true: {cmds:?}"
    );
}

#[tokio::test]
async fn send_session_and_list_commands_emit_expected_types() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.send_session_stats();
    a.send_list_sessions();
    a.send_clear_history();
    let cmds = h.drain_commands().await;
    for expected in [
        "\"type\":\"get_session_stats\"",
        "\"type\":\"list_sessions\"",
        "\"type\":\"clear_history\"",
    ] {
        assert!(
            cmds.iter().any(|c| c.contains(expected)),
            "expected a {expected} command: {cmds:?}"
        );
    }
}

#[tokio::test]
async fn send_resume_session_empty_falls_back_to_list() {
    let mut h = harness().await;
    h.app_mut().send_resume_session("   ");
    h.app_mut().send_resume_session("my-session");
    let cmds = h.drain_commands().await;
    // drain_commands is FIFO: assert positionally so the blank→list and
    // named→resume mapping can't pass if the two were swapped.
    assert_eq!(cmds.len(), 2, "exactly two commands expected: {cmds:?}");
    assert!(
        cmds[0].contains("\"type\":\"list_sessions\""),
        "blank resume name should fall back to list_sessions: {cmds:?}"
    );
    assert!(
        cmds[1].contains("\"type\":\"resume_session\""),
        "named resume should send resume_session: {cmds:?}"
    );
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
    let footer = a.master_session.footer.render(120).join("\n");
    assert!(footer.contains("42"), "{footer}");
}

#[tokio::test]
async fn update_footer_stats_sets_positive_cost_without_context() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_footer_stats(&serde_json::json!({ "cost": 1.25 }));
    assert!(!a.context_stats_requested);
    let footer = a.master_session.footer.render(120).join("\n");
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
    a.master_session.chat.add_entry(ChatEntry::User {
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
    a.master_session.chat.add_entry(ChatEntry::User {
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

#[tokio::test]
async fn model_selector_overlay_renders_with_theme_background() {
    use crate::interface::theme;
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({
        "models": [{
            "id": "openai-api/gpt-5.5",
            "provider": "OpenAI API",
            "auth": null
        }]
    })));
    assert!(a.model_selector.is_some());
    let frame = a.compose_frame();
    let joined = frame.join("\n");
    assert!(
        joined.contains(theme::BG_OVERLAY),
        "model selector overlay should use the theme background"
    );
    assert!(
        joined.contains("Select Model"),
        "frame should contain the model selector title"
    );
    assert!(
        joined.contains("gpt-5.5"),
        "frame should contain a rendered model entry"
    );
    // The overlay region should be a contiguous block of same-width lines.
    let overlay_lines: Vec<&String> = frame
        .iter()
        .filter(|l| l.contains(theme::BG_OVERLAY))
        .collect();
    let first_width = crate::interface::utils::visible_width(overlay_lines[0]);
    for line in &overlay_lines {
        assert_eq!(
            crate::interface::utils::visible_width(line),
            first_width,
            "all overlay lines should have the same width"
        );
    }
}

/// The resume / rewind / model-selector overlays must follow the Quecto theme
/// palette, not render a hardcoded black background. All three share
/// `theme::apply_overlay_bg` / `BG_OVERLAY`, so this asserts the shared overlay
/// background is themed (exercised via the model selector through the real
/// `compose_frame` render path).
#[tokio::test]
async fn overlays_follow_theme_background_not_black() {
    use crate::interface::theme;
    const BLACK_BG: &str = "\x1b[48;2;0;0;0m";

    // The shared overlay background must not be hardcoded black.
    assert_ne!(
        theme::BG_OVERLAY,
        BLACK_BG,
        "overlay background must follow the Quecto theme palette, not hardcoded black"
    );

    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({
        "models": [{ "id": "openai-api/gpt-5.5", "provider": "OpenAI API", "auth": null }]
    })));
    assert!(a.model_selector.is_some());

    let joined = a.compose_frame().join("\n");
    assert!(
        joined.contains(theme::BG_OVERLAY),
        "overlay must paint the theme background"
    );
    assert!(
        !joined.contains(BLACK_BG),
        "overlay must not render a hardcoded black background — it should follow the theme"
    );
    // (b): the modal is delineated by a themed box border, not an opaque fill —
    // the bg is the terminal default so it adapts to the active (light/dark) theme.
    assert!(
        joined.contains('┌') && joined.contains('│') && joined.contains('└'),
        "overlay must be rendered as a themed box border"
    );
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
    a.master_session
        .chat
        .add_entry(ChatEntry::User { text: "x".into() });
    a.context_stats_requested = true;
    a.reset_session("New session");
    assert_eq!(a.master_session.chat.entry_count(), 0);
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
    a.master_session.chat.add_entry(ChatEntry::User {
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
    a.master_session.chat.add_entry(ChatEntry::User {
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
    a.master_session.chat.add_entry(ChatEntry::User {
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

// Sub-agent-first (#820): the sub-agent bar left the below-chat section for the
// always-on left panel. The spinner still renders in the bottom stack, but the
// sub-agent row must NOT — it now lives in the panel (full frame).
#[tokio::test]
async fn spinner_renders_in_bottom_subagent_moved_to_panel() {
    let mut h = harness().await;
    h.event(super::tui_harness::subagents_changed(vec![
        super::tui_harness::subagent("orderworker", "running", None),
    ]));
    // Parent is actively working -> a real spinner line is rendered.
    h.app_mut().spinner = Some(Spinner::new("order-spinner-marker"));

    let bottom: Vec<String> = h
        .app_mut()
        .compose_bottom(120)
        .iter()
        .map(|l| super::app_methods::strip_ansi(l))
        .collect();

    assert!(
        bottom.iter().any(|l| l.contains("order-spinner-marker")),
        "the spinner must still render in the bottom stack: {bottom:?}"
    );
    assert!(
        !bottom.iter().any(|l| l.contains("orderworker")),
        "the sub-agent must NOT render in the bottom stack any more: {bottom:?}"
    );
    let frame = super::app_methods::strip_ansi(&h.app_mut().compose_frame().join("\n"));
    assert!(
        frame.contains("orderworker"),
        "the sub-agent must render in the left panel instead:\n{frame}"
    );
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

#[tokio::test]
async fn main_pane_compact_line_reflects_live_auto_continue_state(// #897 AC2
) {
    // Drive the REAL event→render path: a workflow_state event seeds the bar
    // (which hard-codes auto_continue=false), then the live App toggle must be
    // reflected by the always-visible compact line — not the dead field.
    let mut h = harness().await;
    let wf = serde_json::json!({
        "steps": [{"index": 0, "label": "Build it", "phase": "build", "done": false}],
        "progress": {"done": 0, "total": 1},
        "activeIssue": {"number": 7, "title": "thing"}
    });
    h.app_mut().master_session.workflow_bar = workflow_bar::parse_workflow_event(&wf);

    let now = tokio::time::Instant::now();
    let render = |a: &App| -> String {
        a.render_main_pane_workflow(120, 120, now)
            .iter()
            .map(|l| super::app_methods::strip_ansi(l))
            .collect::<Vec<_>>()
            .join("\n")
    };

    // Default: auto-continue off.
    assert!(
        render(h.app_mut()).contains("auto:off"),
        "{}",
        render(h.app_mut())
    );

    // Drive the REAL response path that updates live auto-continue state; the
    // rendered compact line must follow.
    h.app_mut().handle_response(
        Some("workflow-auto".into()),
        "set_workflow_automation".into(),
        true,
        Some(serde_json::json!({"automation": {"autoContinue": true}})),
        None,
    );
    assert!(
        render(h.app_mut()).contains("auto:on"),
        "{}",
        render(h.app_mut())
    );

    // A subsequent workflow_state rebuild must PRESERVE the live state — the bug
    // was that every event reset the bar field to the hard-coded false. This
    // mirrors handle_workflow_state's (rebuild → mirror_automation_to_bar) flow.
    h.app_mut().master_session.workflow_bar = workflow_bar::parse_workflow_event(&wf);
    h.app_mut().mirror_automation_to_bar();
    assert!(
        render(h.app_mut()).contains("auto:on"),
        "workflow_state rebuild must preserve live auto-continue: {}",
        render(h.app_mut())
    );
}
