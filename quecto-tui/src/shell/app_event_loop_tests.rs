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

#[tokio::test]
async fn run_with_preexisting_exit_flag_performs_startup_and_cleanup() {
    let mut h = harness().await;
    let app = h.app_mut();
    app.should_exit = true;
    assert_eq!(app.run().await, 0);
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
    // A bare ESC byte alone is parsed as Escape key. When idle with an empty
    // editor, that arms the rewind affordance (same as Key::Escape).
    a.process_key_sequence(b"\x1b");
    assert!(
        a.ac().rewind.last_idle_escape.is_some(),
        "bare ESC should be parsed as Escape and arm rewind"
    );
    assert_eq!(a.editor.text(), "", "bare ESC must not insert text");
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
    a.ac_mut().agent_state.start();
    assert!(a.ac().agent_state.is_running());
    a.handle_key(Key::Ctrl('d'));
    assert!(a.should_exit);
    // Abort should have been called (agent_state.abort sets running=false).
    assert!(!a.ac().agent_state.is_running());
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
    a.ac_mut().agent_state.start();
    a.handle_key(Key::Ctrl('c'));
    assert!(
        !a.ac().agent_state.is_running(),
        "Ctrl+C should abort when running and editor empty"
    );
}

#[tokio::test]
async fn handle_key_ctrl_c_noop_when_idle_and_editor_empty() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_key(Key::Ctrl('c'));
    assert!(!a.ac().agent_state.is_running());
    assert_eq!(a.editor.text(), "");
}

#[tokio::test]
async fn handle_key_escape_when_idle_and_editor_empty_arms_rewind() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_key(Key::Escape);
    assert!(
        a.ac().rewind.last_idle_escape.is_some(),
        "first Escape should arm rewind"
    );
}

#[tokio::test]
async fn handle_key_escape_when_running_aborts_agent() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.ac_mut().agent_state.start();
    a.handle_key(Key::Escape);
    assert!(
        !a.ac().agent_state.is_running(),
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
    h.app_mut().handle_key(Key::Ctrl('l'));
    // open_model_selector defers opening until the fresh list arrives, so it
    // sends a list_models request rather than opening the selector synchronously.
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter().any(|c| c.contains("\"type\":\"list_models\"")),
        "Ctrl+L should request the model list: {cmds:?}"
    );
}

#[tokio::test]
async fn handle_key_ctrl_o_toggles_tool_expand() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.ac().master_session.chat.tool_expanded;
    a.handle_key(Key::Ctrl('o'));
    assert_eq!(
        a.ac().master_session.chat.tool_expanded,
        !before,
        "Ctrl+O should toggle tool expand"
    );
}

#[tokio::test]
async fn handle_key_ctrl_o_toggles_active_subagent_tool_expand() {
    // #828: Ctrl+O routes through the ACTIVE session, not always the master.
    let mut h = harness().await;
    let a = h.app_mut();
    a.select_agent(Some("worker")); // lazily creates the child session view
    let m0 = a.ac().master_session.chat.tool_expanded;
    let c0 = a.active_chat_mut().tool_expanded;
    a.handle_key(Key::Ctrl('o'));
    assert_eq!(a.active_chat_mut().tool_expanded, !c0, "toggles child");
    assert_eq!(a.ac().master_session.chat.tool_expanded, m0, "master kept");
}

// The four scroll keys all move `chat.scroll_offset`: PageUp/ScrollUp increase
// it (scroll back into history), PageDown/ScrollDown decrease it (toward the
// latest output, saturating at 0). Table-driven so the four cases share setup.
#[tokio::test]
async fn scroll_keys_move_chat_offset() {
    // (key, scrolls_back) — scrolls_back == true means offset should grow.
    let up_keys = [Key::PageUp, Key::ScrollUp];
    let down_keys = [Key::PageDown, Key::ScrollDown];

    for key in up_keys {
        let label = format!("{key:?}");
        let mut h = harness().await;
        let a = h.app_mut();
        // Seed more content than the viewport so scrolling back is observable
        // (render clamps offset to scrollable range, not just the raw setter).
        for i in 0..20 {
            a.ac_mut().master_session.chat.add_entry(ChatEntry::User {
                text: format!("line {i}"),
            });
        }
        a.ac_mut().master_session.chat.set_viewport_height(3);
        a.ac_mut().master_session.chat.render(80);
        assert_eq!(
            a.ac().master_session.chat.scroll_offset(),
            0,
            "fresh chat is pinned to bottom"
        );
        a.handle_key(key);
        a.ac_mut().master_session.chat.render(80);
        assert!(
            a.ac().master_session.chat.scroll_offset() > 0,
            "{label} should scroll chat back (offset > 0)"
        );
    }

    for key in down_keys {
        let label = format!("{key:?}");
        let mut h = harness().await;
        let a = h.app_mut();
        // First scroll back so there is something to scroll forward from.
        a.ac_mut().master_session.chat.scroll_up(20);
        let before = a.ac().master_session.chat.scroll_offset();
        a.handle_key(key);
        assert!(
            a.ac().master_session.chat.scroll_offset() < before,
            "{label} should scroll chat toward the latest output (offset shrinks)"
        );
    }
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
    // With no slash/@ token active, Tab has nothing to autocomplete and must
    // not insert a literal tab or otherwise mutate the editor.
    a.handle_key(Key::Tab);
    assert_eq!(
        a.editor.text(),
        "",
        "Tab with no active autocomplete should leave the editor empty"
    );
}

#[tokio::test]
async fn handle_key_enter_when_editor_empty_is_noop() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_key(Key::Enter);
    // Empty submit is a no-op.
    assert_eq!(a.ac().master_session.chat.entry_count(), 0);
}

// ── handle_submit: slash commands ──────────────────────────────────────

#[tokio::test]
async fn handle_submit_empty_text_is_noop() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.ac().master_session.chat.entry_count();
    a.handle_submit("   ");
    assert_eq!(a.ac().master_session.chat.entry_count(), before);
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
    a.ac_mut().master_session.chat.add_entry(ChatEntry::User {
        text: "data".into(),
    });
    a.handle_submit("/clear");
    assert_eq!(
        a.ac().master_session.chat.entry_count(),
        0,
        "/clear should clear chat"
    );
}

#[tokio::test]
async fn handle_submit_new_starts_new_session() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.ac_mut().master_session.chat.add_entry(ChatEntry::User {
        text: "data".into(),
    });
    a.handle_submit("/new");
    assert_eq!(
        a.ac().master_session.chat.entry_count(),
        0,
        "/new should clear chat"
    );
    let sent = h.drain_commands().await;
    assert!(
        sent.iter()
            .any(|cmd| cmd.contains("\"type\":\"new_session\""))
    );
}

#[tokio::test]
async fn handle_submit_help_shows_shortcuts() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.ac().master_session.chat.entry_count();
    a.handle_submit("/help");
    assert!(
        a.ac().master_session.chat.entry_count() > before,
        "/help should add a chat entry"
    );
}

#[tokio::test]
async fn handle_submit_hotkeys_shows_shortcuts() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.ac().master_session.chat.entry_count();
    a.handle_submit("/hotkeys");
    assert!(a.ac().master_session.chat.entry_count() > before);
}

#[tokio::test]
async fn handle_submit_unknown_slash_command_notifies() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.ac().master_session.chat.entry_count();
    a.handle_submit("/bogus");
    assert!(
        a.ac().master_session.chat.entry_count() > before,
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
    assert_eq!(
        a.ac().inference.current_model.as_deref(),
        Some("test-model")
    );
}

#[tokio::test]
async fn handle_submit_model_without_name_opens_selector() {
    let mut h = harness().await;
    h.app_mut().handle_submit("/model");
    // `/model` with no argument opens the selector, which defers the open and
    // requests a fresh model list.
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter().any(|c| c.contains("\"type\":\"list_models\"")),
        "/model with no name should request the model list: {cmds:?}"
    );
}

#[tokio::test]
async fn handle_submit_regular_message_adds_user_entry() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.ac().master_session.chat.entry_count();
    a.handle_submit("hello world");
    assert_eq!(
        a.ac().master_session.chat.entry_count(),
        before + 1,
        "should add user message"
    );
}

#[tokio::test]
async fn handle_submit_resume_sends_list_sessions() {
    let mut h = harness().await;
    h.app_mut().handle_submit("/resume");
    // Bare `/resume` lists sessions so the user can pick one.
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter()
            .any(|c| c.contains("\"type\":\"list_sessions\"")),
        "/resume with no name should list sessions: {cmds:?}"
    );
}

#[tokio::test]
async fn handle_submit_resume_with_name_sends_resume() {
    let mut h = harness().await;
    h.app_mut().handle_submit("/resume my-session");
    // `/resume <name>` resumes that named session directly.
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter()
            .any(|c| c.contains("\"type\":\"resume_session\"")),
        "/resume <name> should send a resume_session command: {cmds:?}"
    );
}

#[tokio::test]
async fn handle_submit_workflow_shows_status() {
    let mut h = harness().await;
    let a = h.app_mut();
    let before = a.ac().master_session.chat.entry_count();
    a.handle_submit("/workflow");
    assert!(
        a.ac().master_session.chat.entry_count() > before,
        "/workflow should show status"
    );
}

#[tokio::test]
async fn handle_submit_workflow_auto_sends_toggle_command() {
    let mut h = harness().await;
    let before = h.app_mut().ac().workflow.auto_continue;
    h.app_mut().handle_submit("/workflow-auto");
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter()
            .any(|c| c.contains("\"type\":\"set_workflow_automation\"")),
        "/workflow-auto should send set_workflow_automation: {cmds:?}"
    );
    // The local state is NOT toggled synchronously — it's updated when the
    // server responds. We only verify the command was sent.
    assert_eq!(h.app_mut().ac().workflow.auto_continue, before);
}

#[tokio::test]
async fn handle_submit_workflow_nudge_sends_toggle_command() {
    let mut h = harness().await;
    let before = h.app_mut().ac().workflow.completion_nudge;
    h.app_mut().handle_submit("/workflow-nudge");
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter()
            .any(|c| c.contains("\"type\":\"set_workflow_automation\"")),
        "/workflow-nudge should send set_workflow_automation: {cmds:?}"
    );
    assert_eq!(h.app_mut().ac().workflow.completion_nudge, before);
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
async fn handle_submit_follow_up_command_when_running() {
    let mut h = harness().await;
    h.app_mut().ac_mut().agent_state.start();
    h.app_mut().handle_submit("follow-up message");
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter().any(|c| c.contains("\"type\":\"follow_up\"")),
        "Enter while running should queue a follow-up command: {cmds:?}"
    );
    assert!(
        !cmds
            .iter()
            .any(|c| c.contains("\"streamingBehavior\":\"steer\"")),
        "Enter while running must not claim steer behavior: {cmds:?}"
    );
}

async fn a_handle_submit_text(h: &mut TuiHarness, text: &str) {
    h.app_mut().handle_submit(text);
}

// ── handle_key: Ctrl+Shift combos ──────────────────────────────────────

#[tokio::test]
async fn handle_key_ctrl_shift_a_sends_toggle_command() {
    let mut h = harness().await;
    let before = h.app_mut().ac().workflow.auto_continue;
    h.app_mut().handle_key(Key::CtrlShift('a'));
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter()
            .any(|c| c.contains("\"type\":\"set_workflow_automation\"")),
        "Ctrl+Shift+A should send set_workflow_automation: {cmds:?}"
    );
    // Local state is NOT toggled synchronously — updated on server response.
    assert_eq!(h.app_mut().ac().workflow.auto_continue, before);
}

#[tokio::test]
async fn handle_key_ctrl_shift_n_sends_toggle_command() {
    let mut h = harness().await;
    let before = h.app_mut().ac().workflow.completion_nudge;
    h.app_mut().handle_key(Key::CtrlShift('n'));
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter()
            .any(|c| c.contains("\"type\":\"set_workflow_automation\"")),
        "Ctrl+Shift+N should send set_workflow_automation: {cmds:?}"
    );
    assert_eq!(h.app_mut().ac().workflow.completion_nudge, before);
}

// ── accept_file_mention (tested indirectly via handle_key @files flow) ──

#[tokio::test]
async fn handle_key_at_files_flow_replaces_token_on_tab() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.editor.set_text("see @fi");
    a.workspace.files_autocomplete =
        crate::components::files_autocomplete::FilesAutocomplete::with_files(
            vec!["first.rs".into()],
            4,
        );
    a.refresh_files_autocomplete_from_editor();
    a.handle_key(Key::Tab);
    assert_eq!(a.editor.text(), "see @first.rs ");
}
