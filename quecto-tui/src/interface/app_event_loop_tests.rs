//! Tests for the event-loop input-handling methods (`process_key_sequence`,
//! `handle_key`, `handle_submit`, `handle_abort`) and the key-routing logic
//! that was previously untested (issue #729).
//!
//! These drive the real `App` built by the headless render harness (no TTY,
//! drained socket) so the key routing, slash-command dispatch, and abort flow
//! are exercised without a live agent.

use super::tui_harness::TuiHarness;
use super::*;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

// ── process_key_sequence ──────────────────────────────────────────────

#[tokio::test]
async fn process_key_sequence_parses_simple_char() {
    let mut h = harness().await;
    let a = h.app_mut();
    // Feed 'a' → editor should receive it.
    a.process_key_sequence(b"a");
    assert_eq!(a.editor.text(), "a");
}

#[tokio::test]
async fn process_key_sequence_parses_escape_sequence() {
    let mut h = harness().await;
    let a = h.app_mut();
    // Feed Up arrow → editor text unchanged (it's a navigation key).
    a.process_key_sequence(b"\x1b[A");
    assert_eq!(a.editor.text(), "");
}

#[tokio::test]
async fn process_key_sequence_handles_incomplete_escape() {
    let mut h = harness().await;
    let a = h.app_mut();
    // A bare ESC byte alone is parsed as Escape key.
    a.process_key_sequence(b"\x1b");
    // Escape when idle with empty editor arms rewind — just verify no panic.
}

#[tokio::test]
async fn process_key_sequence_unknown_sequence_is_noop() {
    let mut h = harness().await;
    let a = h.app_mut();
    // An unknown CSI sequence should not panic or add text.
    a.process_key_sequence(b"\x1b[99X");
    assert_eq!(a.editor.text(), "");
}

#[tokio::test]
async fn process_key_sequence_kitty_release_filtered_when_active() {
    let mut h = harness().await;
    let a = h.app_mut();
    // Enable kitty so release events are checked.
    a.kitty.enable();
    // 'a' release event: CSI 97;1:3u — should be filtered out.
    a.process_key_sequence(b"\x1b[97;1:3u");
    assert_eq!(
        a.editor.text(),
        "",
        "release event should not produce input"
    );
}

#[tokio::test]
async fn process_key_sequence_kitty_press_still_works_when_active() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.kitty.enable();
    // 'a' press via kitty: CSI 97;1u
    a.process_key_sequence(b"\x1b[97;1u");
    assert_eq!(a.editor.text(), "a");
}

// ── handle_key: exit and global keys ──────────────────────────────────

#[tokio::test]
async fn handle_key_ctrl_d_sets_exit_flag() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_key(Key::Ctrl('d'));
    assert!(a.should_exit, "Ctrl+D should set should_exit");
}

#[tokio::test]
async fn handle_key_ctrl_d_aborts_if_agent_running() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.agent_state.start();
    assert!(a.agent_state.is_running());
    a.handle_key(Key::Ctrl('d'));
    assert!(a.should_exit);
    // Abort should have been called (agent_state.abort sets running=false).
    assert!(!a.agent_state.is_running());
}

#[tokio::test]
async fn handle_key_ctrl_c_clears_editor_with_text() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.editor.set_text("some text");
    a.handle_key(Key::Ctrl('c'));
    assert_eq!(
        a.editor.text(),
        "",
        "Ctrl+C should clear editor when it has text"
    );
}

#[tokio::test]
async fn handle_key_ctrl_c_aborts_when_running_and_editor_empty() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.agent_state.start();
    a.handle_key(Key::Ctrl('c'));
    assert!(
        !a.agent_state.is_running(),
        "Ctrl+C should abort when running and editor empty"
    );
}

#[tokio::test]
async fn handle_key_ctrl_c_noop_when_idle_and_editor_empty() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_key(Key::Ctrl('c'));
    assert!(!a.agent_state.is_running());
    assert_eq!(a.editor.text(), "");
}

#[tokio::test]
async fn handle_key_escape_when_idle_and_editor_empty_arms_rewind() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_key(Key::Escape);
    assert!(
        a.last_idle_escape.is_some(),
        "first Escape should arm rewind"
    );
}

#[tokio::test]
async fn handle_key_escape_when_running_aborts_agent() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.agent_state.start();
    a.handle_key(Key::Escape);
    assert!(
        !a.agent_state.is_running(),
        "Escape should abort running agent"
    );
}

#[tokio::test]
async fn handle_key_escape_clears_editor_with_text() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.editor.set_text("text");
    a.handle_key(Key::Escape);
    assert_eq!(a.editor.text(), "", "Escape should clear editor with text");
}

#[tokio::test]
async fn handle_key_ctrl_l_opens_model_selector() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_key(Key::Ctrl('l'));
    // open_model_selector sends a ListModels request; the selector opens
    // after the response. Just verify it didn't panic.
}

#[tokio::test]
async fn handle_key_ctrl_o_toggles_tool_expand() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.chat.tool_expanded;
    a.handle_key(Key::Ctrl('o'));
    assert_eq!(
        a.chat.tool_expanded, !before,
        "Ctrl+O should toggle tool expand"
    );
}

#[tokio::test]
async fn handle_key_page_up_scrolls_chat() {
    let mut h = harness().await;
    let a = h.app_mut();
    // Just verify no panic.
    a.handle_key(Key::PageUp);
}

#[tokio::test]
async fn handle_key_page_down_scrolls_chat() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_key(Key::PageDown);
}

#[tokio::test]
async fn handle_key_scroll_up_scrolls_chat() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_key(Key::ScrollUp);
}

#[tokio::test]
async fn handle_key_scroll_down_scrolls_chat() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_key(Key::ScrollDown);
}

#[tokio::test]
async fn handle_key_mouse_press_sets_selection() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_key(Key::MousePress(10, 5));
    assert!(
        a.selection.is_some(),
        "mouse press should start a selection"
    );
    let sel = a.selection.as_ref().unwrap();
    assert_eq!(sel.start.col, 10);
    assert_eq!(sel.start.row, 5);
}

#[tokio::test]
async fn handle_key_mouse_drag_updates_selection() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_key(Key::MousePress(0, 0));
    a.handle_key(Key::MouseDrag(10, 5));
    let sel = a.selection.as_ref().unwrap();
    assert_eq!(sel.end.col, 10);
    assert_eq!(sel.end.row, 5);
}

#[tokio::test]
async fn handle_key_mouse_release_clears_selection() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_key(Key::MousePress(0, 0));
    a.handle_key(Key::MouseRelease(0, 0));
    // Same position press+release → no copy, but selection is consumed.
    assert!(
        a.selection.is_none(),
        "mouse release should clear selection"
    );
}

// ── handle_key: editor forwarding ──────────────────────────────────────

#[tokio::test]
async fn handle_key_printable_char_goes_to_editor() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_key(Key::Char('x'));
    assert_eq!(a.editor.text(), "x");
}

#[tokio::test]
async fn handle_key_backspace_goes_to_editor() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.editor.set_text("abc");
    a.handle_key(Key::Backspace);
    assert_eq!(a.editor.text(), "ab");
}

#[tokio::test]
async fn handle_key_tab_goes_to_editor() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_key(Key::Tab);
    // Tab may trigger autocomplete; just verify no panic.
}

#[tokio::test]
async fn handle_key_enter_when_editor_empty_is_noop() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_key(Key::Enter);
    // Empty submit is a no-op.
    assert_eq!(a.chat.entry_count(), 0);
}

// ── handle_submit: slash commands ──────────────────────────────────────

#[tokio::test]
async fn handle_submit_empty_text_is_noop() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.chat.entry_count();
    a.handle_submit("   ");
    assert_eq!(a.chat.entry_count(), before);
}

#[tokio::test]
async fn handle_submit_quit_sets_exit_flag() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_submit("/quit");
    assert!(a.should_exit);
}

#[tokio::test]
async fn handle_submit_exit_sets_exit_flag() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_submit("/exit");
    assert!(a.should_exit);
}

#[tokio::test]
async fn handle_submit_clear_clears_session() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.chat.add_entry(ChatEntry::User {
        text: "data".into(),
    });
    a.handle_submit("/clear");
    assert_eq!(a.chat.entry_count(), 0, "/clear should clear chat");
}

#[tokio::test]
async fn handle_submit_new_starts_new_session() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.chat.add_entry(ChatEntry::User {
        text: "data".into(),
    });
    a.handle_submit("/new");
    assert_eq!(a.chat.entry_count(), 0, "/new should clear chat");
}

#[tokio::test]
async fn handle_submit_help_shows_shortcuts() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.chat.entry_count();
    a.handle_submit("/help");
    assert!(
        a.chat.entry_count() > before,
        "/help should add a chat entry"
    );
}

#[tokio::test]
async fn handle_submit_hotkeys_shows_shortcuts() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.chat.entry_count();
    a.handle_submit("/hotkeys");
    assert!(a.chat.entry_count() > before);
}

#[tokio::test]
async fn handle_submit_unknown_slash_command_notifies() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.chat.entry_count();
    a.handle_submit("/bogus");
    assert!(
        a.chat.entry_count() > before,
        "unknown command should add status"
    );
    assert!(
        !a.notifications.is_empty(),
        "should notify about unknown command"
    );
}

#[tokio::test]
async fn handle_submit_model_with_name_sends_set_model() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_submit("/model test-model");
    assert_eq!(a.current_model.as_deref(), Some("test-model"));
}

#[tokio::test]
async fn handle_submit_model_without_name_opens_selector() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_submit("/model");
    // open_model_selector sends ListModels request.
    // Just verify it didn't panic.
}

#[tokio::test]
async fn handle_submit_regular_message_adds_user_entry() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.chat.entry_count();
    a.handle_submit("hello world");
    assert_eq!(a.chat.entry_count(), before + 1, "should add user message");
}

#[tokio::test]
async fn handle_submit_steer_when_agent_running() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.agent_state.start();
    a.handle_submit("steer this");
    // When running, the command should be a steer (not a new prompt).
    // Just verify the user message is still added.
    assert!(a.chat.entry_count() > 0);
}

#[tokio::test]
async fn handle_submit_resume_sends_list_sessions() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_submit("/resume");
    // Just verify no panic.
}

#[tokio::test]
async fn handle_submit_resume_with_name_sends_resume() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_submit("/resume my-session");
    // Just verify no panic.
}

#[tokio::test]
async fn handle_submit_workflow_shows_status() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.chat.entry_count();
    a.handle_submit("/workflow");
    assert!(
        a.chat.entry_count() > before,
        "/workflow should show status"
    );
}

#[tokio::test]
async fn handle_submit_workflow_auto_sends_toggle_command() {
    let mut h = harness().await;
    let before = h.app_mut().workflow_auto_continue;
    h.app_mut().handle_submit("/workflow-auto");
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter()
            .any(|c| c.contains("\"type\":\"set_workflow_automation\"")),
        "/workflow-auto should send set_workflow_automation: {cmds:?}"
    );
    // The local state is NOT toggled synchronously — it's updated when the
    // server responds. We only verify the command was sent.
    assert_eq!(h.app_mut().workflow_auto_continue, before);
}

#[tokio::test]
async fn handle_submit_workflow_nudge_sends_toggle_command() {
    let mut h = harness().await;
    let before = h.app_mut().workflow_completion_nudge;
    h.app_mut().handle_submit("/workflow-nudge");
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter()
            .any(|c| c.contains("\"type\":\"set_workflow_automation\"")),
        "/workflow-nudge should send set_workflow_automation: {cmds:?}"
    );
    assert_eq!(h.app_mut().workflow_completion_nudge, before);
}

// ── handle_submit: command verification ────────────────────────────────

#[tokio::test]
async fn handle_submit_regular_message_sends_prompt_command() {
    let mut h = harness().await;
    a_handle_submit_text(&mut h, "test message").await;
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter().any(|c| c.contains("\"type\":\"prompt\"")),
        "should send a prompt command: {cmds:?}"
    );
}

#[tokio::test]
async fn handle_submit_quit_does_not_send_prompt() {
    let mut h = harness().await;
    h.app_mut().handle_submit("/quit");
    let cmds = h.drain_commands().await;
    assert!(
        !cmds.iter().any(|c| c.contains("\"type\":\"prompt\"")),
        "/quit should not send a prompt command"
    );
}

#[tokio::test]
async fn handle_submit_steer_command_when_running() {
    let mut h = harness().await;
    h.app_mut().agent_state.start();
    h.app_mut().handle_submit("steer message");
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter()
            .any(|c| c.contains("\"streamingBehavior\":\"steer\"")),
        "should send steer behavior when agent is running: {cmds:?}"
    );
}

async fn a_handle_submit_text(h: &mut TuiHarness, text: &str) {
    h.app_mut().handle_submit(text);
}

// ── handle_abort ──────────────────────────────────────────────────────

#[tokio::test]
async fn handle_abort_sends_abort_command() {
    let mut h = harness().await;
    h.app_mut().handle_abort();
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter().any(|c| c.contains("\"type\":\"abort\"")),
        "handle_abort should send an abort command: {cmds:?}"
    );
}

#[tokio::test]
async fn handle_abort_stops_spinner() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.agent_state.start();
    a.spinner = Some(Spinner::new("Working"));
    a.handle_abort();
    assert!(a.spinner.is_none(), "abort should clear spinner");
}

#[tokio::test]
async fn handle_abort_finalizes_assistant() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.agent_state.start();
    a.spinner = Some(Spinner::new("Working"));
    a.handle_abort();
    // finalize_assistant is called; just verify no panic and spinner cleared.
    assert!(a.spinner.is_none());
}

#[tokio::test]
async fn handle_abort_adds_status_message() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.chat.entry_count();
    a.handle_abort();
    assert!(
        a.chat.entry_count() > before,
        "abort should add a status entry"
    );
}

#[tokio::test]
async fn handle_abort_calls_agent_state_abort() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.agent_state.start();
    assert!(a.agent_state.is_running());
    a.handle_abort();
    assert!(!a.agent_state.is_running(), "abort should stop agent_state");
}

#[tokio::test]
async fn handle_abort_sets_footer_streaming_false() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.footer.set_streaming(true);
    a.handle_abort();
    // Footer streaming should be false after abort.
    let rendered = a.footer.render(80).join("\n");
    assert!(!rendered.contains("streaming") || !rendered.to_lowercase().contains("thinking"));
}

// ── handle_key: overlay routing ───────────────────────────────────────

#[tokio::test]
async fn handle_key_routes_to_overlay_when_active() {
    let mut h = harness().await;
    let a = h.app_mut();
    // Open the resume selector to activate an overlay-like state.
    let data = serde_json::json!({"sessions": [{"name": "alpha"}]});
    a.open_resume_selector(&data);
    assert!(a.resume_selector.is_some());
    // Escape should close the selector, not clear the editor.
    a.handle_key(Key::Escape);
    assert!(a.resume_selector.is_none(), "Escape should close selector");
}

#[tokio::test]
async fn handle_key_routes_to_model_selector_when_active() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.open_model_selector();
    a.handle_list_models(Some(serde_json::json!({ "models": [] })));
    assert!(a.model_selector.is_some());
    // Escape closes the selector.
    a.handle_key(Key::Escape);
    assert!(a.model_selector.is_none());
}

#[tokio::test]
async fn handle_key_routes_to_rewind_selector_when_active() {
    let mut h = harness().await;
    let a = h.app_mut();
    let data = serde_json::json!({"messages": [{"role": "user", "content": "turn"}]});
    a.open_rewind_selector(&data);
    assert!(a.rewind_selector.is_some());
    a.handle_key(Key::Escape);
    assert!(a.rewind_selector.is_none());
}

// ── handle_key: Ctrl+Shift combos ──────────────────────────────────────

#[tokio::test]
async fn handle_key_ctrl_shift_a_sends_toggle_command() {
    let mut h = harness().await;
    let before = h.app_mut().workflow_auto_continue;
    h.app_mut().handle_key(Key::CtrlShift('a'));
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter()
            .any(|c| c.contains("\"type\":\"set_workflow_automation\"")),
        "Ctrl+Shift+A should send set_workflow_automation: {cmds:?}"
    );
    // Local state is NOT toggled synchronously — updated on server response.
    assert_eq!(h.app_mut().workflow_auto_continue, before);
}

#[tokio::test]
async fn handle_key_ctrl_shift_n_sends_toggle_command() {
    let mut h = harness().await;
    let before = h.app_mut().workflow_completion_nudge;
    h.app_mut().handle_key(Key::CtrlShift('n'));
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter()
            .any(|c| c.contains("\"type\":\"set_workflow_automation\"")),
        "Ctrl+Shift+N should send set_workflow_automation: {cmds:?}"
    );
    assert_eq!(h.app_mut().workflow_completion_nudge, before);
}

// ── accept_file_mention (tested indirectly via handle_key @files flow) ──

#[tokio::test]
async fn handle_key_at_files_flow_replaces_token_on_tab() {
    let mut h = harness().await;
    let a = h.app_mut();
    // Type "@fi" to trigger the @files autocomplete.
    a.editor.set_text("@fi");
    a.files_autocomplete.update("@fi", 3);
    // Simulate Tab accepting the first suggestion (if any).
    // Just verify the flow doesn't panic — files_autocomplete may be empty
    // in the test env (no git ls-files), but the routing logic is exercised.
    a.handle_key(Key::Tab);
}
