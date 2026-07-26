use super::*;

#[test]
fn enter_tui_restores_sgr_mouse_reporting_for_scroll_and_selection() {
    assert!(ENTER_TUI.contains("\x1b[?1006h"));
    assert!(ENTER_TUI.contains("\x1b[?1002h"));
    assert!(
        !ENTER_TUI.contains("?1000h"),
        "button-event mode is enough; avoid broader basic click tracking"
    );
}

#[test]
fn exit_tui_still_disables_legacy_mouse_modes() {
    assert!(EXIT_TUI.contains("\x1b[?1006l"));
    assert!(EXIT_TUI.contains("\x1b[?1002l"));
    assert!(EXIT_TUI.contains("\x1b[?1000l"));
}

#[test]
fn terminal_size_is_reasonable() {
    let (w, h) = get_terminal_size();
    assert!(w > 0);
    assert!(h > 0);
}

#[test]
fn terminal_new_has_dimensions() {
    let term = Terminal::new();
    assert!(term.width > 0);
    assert!(term.height > 0);
}

#[test]
fn terminal_default_matches_new() {
    let term = Terminal::default();
    assert!(term.width > 0);
    assert!(term.height > 0);
}

#[test]
fn refresh_size_keeps_positive_dimensions() {
    let mut term = Terminal::new();
    term.width = 1;
    term.height = 1;
    term.refresh_size();
    assert!(term.width > 0);
    assert!(term.height > 0);
}

#[test]
fn write_helpers_do_not_panic() {
    let term = Terminal::new();
    // Benign byte writes — no screen-altering escapes.
    term.write(b"");
    term.write_str("");
    // hide+show cursor is a visual no-op pair.
    term.hide_cursor();
    term.show_cursor();
}

#[test]
fn exit_raw_mode_without_enter_is_noop() {
    let mut term = Terminal::new();
    // No saved termios → exit is a no-op and must not panic.
    term.exit_raw_mode();
    assert!(term.saved.is_none());
}

// ── enter_raw_mode behavior ─────────────────────────────────────

#[test]
fn enter_raw_mode_is_idempotent() {
    // Calling enter_raw_mode twice should not panic and should not
    // overwrite the saved state (the second call is a no-op).
    let mut term = Terminal::new();
    term.enter_raw_mode();
    let first_saved = term.saved.is_some();
    term.enter_raw_mode(); // second call — should be a no-op
    assert_eq!(
        term.saved.is_some(),
        first_saved,
        "second enter_raw_mode should not change saved state"
    );
    term.exit_raw_mode();
}

#[test]
fn enter_then_exit_restores_no_saved_state() {
    let mut term = Terminal::new();
    term.enter_raw_mode();
    // In a test env (non-TTY) enter_raw_mode may not save anything,
    // but exit must still be safe.
    term.exit_raw_mode();
    assert!(term.saved.is_none());
}

#[test]
fn drop_after_enter_raw_mode_does_not_panic() {
    // Drop calls exit_raw_mode + show_cursor; must not panic.
    let mut term = Terminal::new();
    term.enter_raw_mode();
    drop(term);
}

// ── write helpers ───────────────────────────────────────────────

#[test]
fn write_str_emits_bytes() {
    let term = Terminal::new();
    // Just verify no panic on a benign string.
    term.write_str("test");
}

#[test]
fn clear_screen_does_not_panic() {
    let term = Terminal::new();
    term.clear_screen();
}

// ── refresh_size ────────────────────────────────────────────────

#[test]
fn refresh_size_updates_both_dimensions() {
    let mut term = Terminal::new();
    term.width = 1;
    term.height = 1;
    term.refresh_size();
    // After refresh, dimensions should match the real terminal
    // (or the 80×24 fallback in a non-TTY environment).
    assert!(term.width >= 80, "width should be at least the fallback");
    assert!(term.height >= 24, "height should be at least the fallback");
}

#[test]
fn manual_dimension_override_persists_until_refresh() {
    let mut term = Terminal::new();
    term.width = 200;
    term.height = 50;
    assert_eq!(term.width, 200);
    assert_eq!(term.height, 50);
    term.refresh_size();
    // After refresh the real/fallback size takes over.
    assert!(term.width != 200 || term.height != 50);
}
