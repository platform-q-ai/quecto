//! Region-coverage tests for `app_methods`, `app_rewind`, and `app_response`.

use super::app_selection::{SelectionAnchor, TextSelection};
use super::tui_harness::TuiHarness;
use super::*;

const MODEL_ID: &str = "anthropic/claude-opus-4-5";

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

fn chat_text(app: &mut App) -> String {
    app.conn
        .master_session
        .chat
        .render(120)
        .iter()
        .map(|l| super::app_render_helpers::strip_ansi(l))
        .collect::<Vec<_>>()
        .join("\n")
}

fn command_has_string_fields(command: &str, expected: &[(&str, &str)]) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(command) else {
        return false;
    };

    expected.iter().all(|(field, expected_value)| {
        value.get(*field).and_then(|v| v.as_str()) == Some(*expected_value)
    })
}

// ── app_methods: slash-command handlers ──────────────────────────────

#[tokio::test]
async fn reject_unknown_slash_command_adds_status_and_notifies() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.conn.master_session.chat.entry_count();
    a.reject_unknown_slash_command("/bogus");
    assert_eq!(a.conn.master_session.chat.entry_count(), before + 1);
    assert!(!a.notifications.is_empty() && chat_text(a).contains("/bogus"));
}

#[tokio::test]
async fn show_help_appends_shortcut_status() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.show_help();
    let t = chat_text(a);
    // #1179: Shift+click opens OSC 8 links under mouse capture.
    assert!(
        t.contains("Keyboard shortcuts")
            && t.contains("/resume")
            && t.contains("Shift+click")
            && t.contains("OSC 8"),
        "{t}"
    );
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
    h.app_mut().conn.master_session.workflow_bar = workflow_bar::parse_workflow_event(&wf);
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
    h.app_mut().conn.master_session.workflow_bar = workflow_bar::parse_workflow_event(&wf);
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
    assert!(a.conn.sessions.context_stats_requested);
    assert!(chat_text(a).contains("Session: cli:foo"));
}

#[tokio::test]
async fn show_session_stats_without_context_leaves_flag_false() {
    let mut h = harness().await;
    let data = serde_json::json!({"sessionKey": "cli:bar"});
    let a = h.app_mut();
    a.show_session_stats(&data);
    assert!(!a.conn.sessions.context_stats_requested);
}

#[tokio::test]
async fn send_set_model_records_current_model() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.conn.sessions.context_stats_requested = true;
    a.send_set_model(MODEL_ID);
    assert_eq!(a.conn.inference.current_model.as_deref(), Some(MODEL_ID));
    assert!(!a.conn.sessions.context_stats_requested);
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
    assert!(a.conn.sessions.context_stats_requested);
    let footer = a.conn.master_session.footer.render(120).join("\n");
    assert!(footer.contains("42"), "{footer}");
}

#[tokio::test]
async fn update_footer_stats_ignores_positive_cost_without_context() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.update_footer_stats(&serde_json::json!({ "cost": 1.25 }));
    assert!(!a.conn.sessions.context_stats_requested);
    let footer = a.conn.master_session.footer.render(120).join("\n");
    assert!(!footer.contains("$"), "{footer}");
}

// ── app_methods: resume selector ─────────────────────────────────────

#[tokio::test]
async fn open_resume_selector_empty_shows_status_no_selector() {
    let mut h = harness().await;
    let data = serde_json::json!({"sessions": []});
    let a = h.app_mut();
    a.open_resume_selector(&data);
    assert!(a.conn.sessions.resume_selector.is_none());
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
    assert_eq!(
        a.conn
            .sessions
            .resume_selector
            .as_ref()
            .unwrap()
            .item_count(),
        2
    );
}

#[tokio::test]
async fn open_resume_selector_without_names_shows_status() {
    let mut h = harness().await;
    let data = serde_json::json!({"sessions": [{"messageCount": 1}]});
    let a = h.app_mut();
    a.open_resume_selector(&data);
    assert!(a.conn.sessions.resume_selector.is_none());
    assert!(chat_text(a).contains("No resumable"));
}

#[tokio::test]
async fn handle_resume_selector_key_enter_selects_and_closes() {
    let mut h = harness().await;
    let data = serde_json::json!({"sessions": [{"name": "alpha", "messageCount": 3}]});
    let a = h.app_mut();
    a.open_resume_selector(&data);
    a.handle_resume_selector_key(&Key::Enter);
    assert!(a.conn.sessions.resume_selector.is_none());
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter().any(|c| command_has_string_fields(
            c,
            &[("type", "resume_session"), ("session", "alpha")]
        )),
        "Enter should send resume_session for selected session: {cmds:?}"
    );
}

#[tokio::test]
async fn handle_resume_selector_key_escape_cancels() {
    let mut h = harness().await;
    let data = serde_json::json!({"sessions": [{"name": "alpha"}]});
    let a = h.app_mut();
    a.open_resume_selector(&data);
    a.handle_resume_selector_key(&Key::Escape);
    assert!(a.conn.sessions.resume_selector.is_none());
}

#[tokio::test]
async fn handle_resume_selector_key_pending_keeps_selector() {
    let mut h = harness().await;
    let data = serde_json::json!({"sessions": [{"name": "a"}, {"name": "b"}]});
    let a = h.app_mut();
    a.open_resume_selector(&data);
    a.handle_resume_selector_key(&Key::Down);
    assert!(a.conn.sessions.resume_selector.is_some());
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
    assert!(!text.contains("Session resumed"));
}

#[tokio::test]
async fn replace_chat_with_messages_missing_messages_preserves_chat_and_reports_error() {
    let mut h = harness().await;
    let data = serde_json::json!({});
    let a = h.app_mut();
    a.conn.master_session.chat.add_entry(ChatEntry::User {
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
        .map(|line| super::app_render_helpers::strip_ansi(line))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(notification_text.contains("Invalid resume payload"));
}

#[tokio::test]
async fn replace_chat_with_messages_non_array_messages_preserves_chat_and_reports_error() {
    let mut h = harness().await;
    let data = serde_json::json!({"messages": "bad"});
    let a = h.app_mut();
    a.conn.master_session.chat.add_entry(ChatEntry::User {
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
    assert!(a.inference.model_selector.is_some());
    a.handle_model_selector_key(&Key::Escape);
    assert!(a.inference.model_selector.is_none());
}

#[tokio::test]
async fn model_selector_enter_selects_and_sets_model() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({
        "models": [{ "id": "openai-api/gpt-5.5", "provider": "OpenAI API" }]
    })));
    a.handle_model_selector_key(&Key::Enter);
    assert!(a.inference.model_selector.is_none());
    assert_eq!(
        a.conn.inference.current_model.as_deref(),
        Some("openai-api/gpt-5.5")
    );
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter().any(|c| command_has_string_fields(
            c,
            &[("type", "set_model"), ("model", "openai-api/gpt-5.5")]
        )),
        "Enter should send set_model for selected model: {cmds:?}"
    );
}

#[tokio::test]
async fn model_selector_pending_keeps_open() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({ "models": [] })));
    a.handle_model_selector_key(&Key::Down);
    assert!(a.inference.model_selector.is_some());
}

#[tokio::test]
async fn model_selector_overlay_renders_with_theme_background() {
    use crate::components::theme;
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
    assert!(a.inference.model_selector.is_some());
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
    let first_width = crate::components::utils::visible_width(overlay_lines[0]);
    for line in &overlay_lines {
        assert_eq!(
            crate::components::utils::visible_width(line),
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
    use crate::components::theme;
    const BLACK_BG: &str = "\x1b[48;2;0;0;0m";

    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({
        "models": [{ "id": "openai-api/gpt-5.5", "provider": "OpenAI API", "auth": null }]
    })));
    assert!(a.inference.model_selector.is_some());

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
    a.conn
        .master_session
        .chat
        .add_entry(ChatEntry::User { text: "x".into() });
    a.conn.sessions.context_stats_requested = true;
    a.reset_session("New session");
    assert_eq!(a.conn.master_session.chat.entry_count(), 0);
    assert!(!a.conn.sessions.context_stats_requested);
    assert!(!a.notifications.is_empty());
}

#[tokio::test]
async fn extract_selection_spans_rows_and_normalizes_order() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.clear_panel_for_tests();
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
    a.clear_panel_for_tests();
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
    a.conn.master_session.chat.add_entry(ChatEntry::User {
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
    a.conn.master_session.chat.add_entry(ChatEntry::User {
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
    a.conn.master_session.chat.add_entry(ChatEntry::User {
        text: "hello world".into(),
    });
    // Simulate press+drag keeping the selection live, then a frame renders.
    a.selection = Some(TextSelection {
        start: SelectionAnchor { col: 0, row: 0 },
        end: SelectionAnchor {
            col: 80,
            row: a.terminal.height as u16,
        },
    });
    let _ = a.compose_frame();
    // The extraction buffer captured during the live drag must let a copy
    // recover visible text even though the optimization skips idle frames.
    let start = SelectionAnchor { col: 0, row: 0 };
    let end = SelectionAnchor {
        col: 80,
        row: a.terminal.height as u16,
    };
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
    let frame = h.app_mut().compose_frame().join("\n");
    assert!(frame.contains("Resume session"));
    assert!(frame.contains("alpha"));
}

#[tokio::test]
async fn compose_frame_with_model_overlay() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({
        "models": [{ "id": "openai-api/gpt-5.5", "provider": "OpenAI API" }]
    })));
    let frame = a.compose_frame().join("\n");
    assert!(frame.contains("Select Model"));
    assert!(frame.contains("gpt-5.5"));
}

#[tokio::test]
async fn compose_frame_with_rewind_overlay() {
    let mut h = harness().await;
    let data = serde_json::json!({
        "messages": [{"id": "m1", "role": "user", "content": "first turn"}]
    });
    h.app_mut().open_rewind_selector(&data);
    let frame = h.app_mut().compose_frame().join("\n");
    assert!(frame.contains("Go back to"));
    assert!(frame.contains("first turn"));
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
    h.app_mut().conn.spinner = Some(Spinner::new("order-spinner-marker"));

    let bottom: Vec<String> = h
        .app_mut()
        .compose_bottom(120)
        .iter()
        .map(|l| super::app_render_helpers::strip_ansi(l))
        .collect();

    assert!(
        bottom.iter().any(|l| l.contains("order-spinner-marker")),
        "the spinner must still render in the bottom stack: {bottom:?}"
    );
    assert!(
        !bottom.iter().any(|l| l.contains("orderworker")),
        "the sub-agent must NOT render in the bottom stack any more: {bottom:?}"
    );
    let frame = super::app_render_helpers::strip_ansi(&h.app_mut().compose_frame().join("\n"));
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
    let one = super::app_render_helpers::strip_ansi(
        &super::app_render_helpers::subagent_activity_line(1, 0),
    );
    let many = super::app_render_helpers::strip_ansi(
        &super::app_render_helpers::subagent_activity_line(2, 999),
    );
    assert!(one.contains("1 subagent working") && many.contains("2 subagents working"));
}

#[test]
fn strip_ansi_handles_csi_osc_and_plain() {
    use super::app_render_helpers::strip_ansi;
    assert_eq!(strip_ansi("plain"), "plain");
    assert_eq!(strip_ansi("\x1b[31mred\x1b[0m"), "red");
    assert_eq!(strip_ansi("\x1b]0;title\x07body"), "body");
    assert_eq!(strip_ansi("\x1b]8;;url\x1b\\link"), "link");
}
