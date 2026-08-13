//! #1277 characterization: app-level text-input integration.
//!
//! Pins editor↔autocomplete update, Enter submit/clear/history, and slash
//! autocomplete Enter accept through the real App handle_key path.

use super::tui_harness::TuiHarness;
use super::*;

async fn harness() -> TuiHarness {
    TuiHarness::new().await
}

// ── #1277 characterization: app-level text-input integration ───────────

#[tokio::test]
async fn handle_key_editor_input_updates_slash_autocomplete_from_editor_text() {
    let mut h = harness().await;
    let a = h.app_mut();
    // Typing a slash prefix must activate slash autocomplete from editor text.
    a.handle_key(Key::Char('/'));
    a.handle_key(Key::Char('h'));
    a.handle_key(Key::Char('e'));
    assert!(
        a.autocomplete.is_active(),
        "slash prefix must open autocomplete after editor keys"
    );
    assert!(
        a.autocomplete.suggestion_count() > 0,
        "suggestions must reflect editor text"
    );
    // Backspace that leaves the slash should keep it active; leaving slash dismisses.
    a.handle_key(Key::Backspace);
    a.handle_key(Key::Backspace);
    a.handle_key(Key::Backspace);
    assert_eq!(a.editor.text(), "");
    assert!(
        !a.autocomplete.is_active(),
        "clearing the slash draft must dismiss autocomplete"
    );
}

#[tokio::test]
async fn handle_key_enter_with_nonempty_editor_submits_prompt_and_clears() {
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_key(Key::Char('h'));
    a.handle_key(Key::Char('i'));
    a.handle_key(Key::Enter);
    assert_eq!(
        a.editor.text(),
        "",
        "Enter must clear the draft after submit"
    );
    assert!(
        a.conn.master_session.chat.entry_count() >= 1,
        "Enter must dispatch handle_submit (user entry appears)"
    );
    let cmds = h.drain_commands().await;
    assert!(
        cmds.iter().any(|c| c.contains("\"type\":\"prompt\"")),
        "non-empty Enter must send a prompt command: {cmds:?}"
    );
    // History recorded: Up restores the submitted text.
    h.app_mut().handle_key(Key::Up);
    assert_eq!(h.app_mut().editor.text(), "hi");
}

#[tokio::test]
async fn slash_autocomplete_enter_accepts_submits_history_and_clears() {
    // Contract: set_text(value), add_to_history(trim), dismiss, handle_submit, set_text("").
    // Drive real keys so the full App path is exercised (not handle_submit alone).
    let mut h = harness().await;
    let a = h.app_mut();
    a.handle_key(Key::Char('/'));
    a.handle_key(Key::Char('q'));
    a.handle_key(Key::Char('u'));
    assert!(a.autocomplete.is_active());
    let selected = a
        .autocomplete
        .selected_value()
        .expect("slash autocomplete should highlight a suggestion");
    assert!(
        selected.starts_with("/qu") || selected == "/quit",
        "expected /quit-class selection, got {selected:?}"
    );
    a.handle_key(Key::Enter);
    assert_eq!(
        a.editor.text(),
        "",
        "after slash Enter the draft must be cleared"
    );
    assert!(
        !a.autocomplete.is_active(),
        "autocomplete must dismiss after Enter accept"
    );
    // /quit sets exit; history still records the accepted value.
    assert!(a.should_exit, "/quit accept must run handle_submit");
    a.handle_key(Key::Up);
    assert_eq!(
        a.editor.text(),
        selected.trim(),
        "history must store the accepted (trimmed) suggestion"
    );
}
