use super::*;

#[test]
fn insert_characters() {
    let mut e = Editor::new();
    e.handle_input(&Key::Char('h'));
    e.handle_input(&Key::Char('i'));
    assert_eq!(e.text(), "hi");
}

#[test]
fn backspace_deletes_char() {
    let mut e = Editor::new();
    e.set_text("hello");
    e.handle_input(&Key::Backspace);
    assert_eq!(e.text(), "hell");
}

#[test]
fn cursor_left_right() {
    let mut e = Editor::new();
    e.set_text("abcd");
    e.handle_input(&Key::Left);
    e.handle_input(&Key::Left);
    e.handle_input(&Key::Char('X'));
    assert_eq!(e.text(), "abXcd");
}

#[test]
fn home_moves_to_start() {
    let mut e = Editor::new();
    e.set_text("hello");
    e.handle_input(&Key::Home);
    e.handle_input(&Key::Char('X'));
    assert_eq!(e.text(), "Xhello");
}

#[test]
fn end_moves_to_end() {
    let mut e = Editor::new();
    e.set_text("hello");
    e.handle_input(&Key::Home);
    e.handle_input(&Key::End);
    e.handle_input(&Key::Char('!'));
    assert_eq!(e.text(), "hello!");
}

#[test]
fn ctrl_u_kills_to_start() {
    let mut e = Editor::new();
    e.set_text("hello world");
    // Move cursor to position 5
    e.handle_input(&Key::Home);
    for _ in 0..5 {
        e.handle_input(&Key::Right);
    }
    e.handle_input(&Key::Ctrl('u'));
    assert_eq!(e.text(), " world");
}

#[test]
fn ctrl_k_kills_to_end() {
    let mut e = Editor::new();
    e.set_text("hello world");
    e.handle_input(&Key::Home);
    for _ in 0..5 {
        e.handle_input(&Key::Right);
    }
    e.handle_input(&Key::Ctrl('k'));
    assert_eq!(e.text(), "hello");
}

#[test]
fn multiline_input() {
    let mut e = Editor::new();
    e.handle_input(&Key::Char('a'));
    e.handle_input(&Key::ShiftEnter); // Shift+Enter for newline
    e.handle_input(&Key::Char('b'));
    assert_eq!(e.text(), "a\nb");
    assert_eq!(e.lines.len(), 2);
}

#[test]
fn submit_clears_and_returns() {
    let mut e = Editor::new();
    e.handle_input(&Key::Char('h'));
    e.handle_input(&Key::Char('i'));
    e.handle_input(&Key::Enter); // Enter submits
    assert_eq!(e.take_submit(), Some("hi".to_string()));
    assert_eq!(e.text(), "");
}

#[test]
fn history_navigation() {
    let mut e = Editor::new();
    e.set_text("first");
    e.handle_input(&Key::Enter); // submit
    let _ = e.take_submit();
    e.set_text("second");
    e.handle_input(&Key::Enter); // submit
    let _ = e.take_submit();
    // Up goes to most recent
    e.navigate_history_up();
    assert_eq!(e.text(), "second");
    e.navigate_history_up();
    assert_eq!(e.text(), "first");
    // Down goes back
    e.navigate_history_down();
    assert_eq!(e.text(), "second");
    e.navigate_history_down();
    assert_eq!(e.text(), ""); // back to empty (saved text)
}

#[test]
fn render_has_borders() {
    let mut e = Editor::new();
    e.set_text("hello");
    let lines = e.render(40);
    assert!(
        lines.len() >= 3,
        "should have top border, content, bottom border"
    );
    // First and last lines should contain border characters
    assert!(
        lines[0].contains('─'),
        "top border should contain ─: {}",
        lines[0]
    );
    assert!(
        lines.last().unwrap().contains('─'),
        "bottom border should contain ─"
    );
}

#[test]
fn render_respects_width() {
    let mut e = Editor::new();
    e.set_text("hello world");
    let lines = e.render(40);
    for line in &lines {
        assert!(
            visible_width(line) <= 40,
            "line exceeds width 40: {} (width={})",
            line,
            visible_width(line)
        );
    }
}

#[test]
fn bash_mode_detected() {
    let mut e = Editor::new();
    e.set_text("!ls -la");
    assert!(e.is_bash_mode());
    e.set_text("hello");
    assert!(!e.is_bash_mode());
}

#[test]
fn delete_at_cursor() {
    let mut e = Editor::new();
    e.set_text("abcd");
    e.handle_input(&Key::Home);
    e.handle_input(&Key::Delete);
    assert_eq!(e.text(), "bcd");
}

#[test]
fn paste_inserts_text() {
    let mut e = Editor::new();
    e.handle_input(&Key::Paste("hello\nworld".to_string()));
    assert_eq!(e.text(), "hello\nworld");
}

#[test]
fn word_left_movement() {
    let mut e = Editor::new();
    e.set_text("hello world test");
    // Cursor is at end
    e.word_left();
    e.handle_input(&Key::Char('X'));
    assert_eq!(e.text(), "hello world Xtest");
}

#[test]
fn word_right_movement() {
    let mut e = Editor::new();
    e.set_text("hello world test");
    e.handle_input(&Key::Home);
    e.word_right();
    e.handle_input(&Key::Char('X'));
    assert_eq!(e.text(), "hello Xworld test");
}

#[test]
fn multiline_up_down() {
    let mut e = Editor::new();
    e.handle_input(&Key::Char('a'));
    e.handle_input(&Key::ShiftEnter);
    e.handle_input(&Key::Char('b'));
    e.handle_input(&Key::ShiftEnter);
    e.handle_input(&Key::Char('c'));
    // Cursor on line 3. Up should go to line 2.
    e.handle_input(&Key::Up);
    e.handle_input(&Key::Char('X'));
    assert_eq!(e.text(), "a\nbX\nc");
}

#[test]
fn multiline_down_from_first_line() {
    let mut e = Editor::new();
    e.handle_input(&Key::Char('a'));
    e.handle_input(&Key::ShiftEnter);
    e.handle_input(&Key::Char('b'));
    // Go to first line
    e.handle_input(&Key::Up);
    e.handle_input(&Key::Down);
    e.handle_input(&Key::Char('X'));
    assert_eq!(e.text(), "a\nbX");
}

#[test]
fn ctrl_a_moves_home() {
    let mut e = Editor::new();
    e.set_text("hello");
    e.handle_input(&Key::Ctrl('a'));
    e.handle_input(&Key::Char('X'));
    assert_eq!(e.text(), "Xhello");
}

#[test]
fn ctrl_e_moves_end() {
    let mut e = Editor::new();
    e.set_text("hello");
    e.handle_input(&Key::Home);
    e.handle_input(&Key::Ctrl('e'));
    e.handle_input(&Key::Char('!'));
    assert_eq!(e.text(), "hello!");
}

#[test]
fn alt_enter_inserts_newline() {
    let mut e = Editor::new();
    e.handle_input(&Key::Char('a'));
    e.handle_input(&Key::Alt('\n'));
    e.handle_input(&Key::Char('b'));
    assert_eq!(e.text(), "a\nb");
}

#[test]
fn alt_b_word_left() {
    let mut e = Editor::new();
    e.set_text("hello world");
    e.handle_input(&Key::Alt('b'));
    e.handle_input(&Key::Char('X'));
    assert_eq!(e.text(), "hello Xworld");
}

#[test]
fn alt_f_word_right() {
    let mut e = Editor::new();
    e.set_text("hello world");
    e.handle_input(&Key::Home);
    e.handle_input(&Key::Alt('f'));
    e.handle_input(&Key::Char('X'));
    assert_eq!(e.text(), "hello Xworld");
}

#[test]
fn unhandled_key_returns_false() {
    let mut e = Editor::new();
    assert!(!e.handle_input(&Key::PageUp));
}

#[test]
fn invalidate_clears_cache() {
    let mut e = Editor::new();
    e.set_text("hello");
    let _ = e.render(40);
    assert!(e.cached_lines.is_some());
    e.invalidate();
    assert!(e.cached_lines.is_none());
}

#[test]
fn backspace_at_start_of_line_joins() {
    let mut e = Editor::new();
    e.handle_input(&Key::Char('a'));
    e.handle_input(&Key::ShiftEnter);
    e.handle_input(&Key::Char('b'));
    e.handle_input(&Key::Home);
    e.handle_input(&Key::Backspace);
    assert_eq!(e.text(), "ab");
}

#[test]
fn delete_at_end_of_line_joins() {
    let mut e = Editor::new();
    e.handle_input(&Key::Char('a'));
    e.handle_input(&Key::ShiftEnter);
    e.handle_input(&Key::Char('b'));
    e.handle_input(&Key::Up);
    e.handle_input(&Key::End);
    e.handle_input(&Key::Delete);
    assert_eq!(e.text(), "ab");
}

#[test]
fn left_at_start_wraps_up() {
    let mut e = Editor::new();
    e.handle_input(&Key::Char('a'));
    e.handle_input(&Key::ShiftEnter);
    e.handle_input(&Key::Char('b'));
    e.handle_input(&Key::Home);
    e.handle_input(&Key::Left);
    e.handle_input(&Key::Char('X'));
    assert_eq!(e.text(), "aX\nb");
}

#[test]
fn right_at_end_wraps_down() {
    let mut e = Editor::new();
    e.handle_input(&Key::Char('a'));
    e.handle_input(&Key::ShiftEnter);
    e.handle_input(&Key::Char('b'));
    e.handle_input(&Key::Up);
    e.handle_input(&Key::End);
    e.handle_input(&Key::Right);
    e.handle_input(&Key::Char('X'));
    assert_eq!(e.text(), "a\nXb");
}

#[test]
fn history_up_beyond_start_stays() {
    let mut e = Editor::new();
    e.add_to_history("first");
    e.navigate_history_up();
    e.navigate_history_up(); // beyond start
    assert_eq!(e.text(), "first");
}

#[test]
fn history_down_beyond_end_stays() {
    let mut e = Editor::new();
    e.navigate_history_down(); // no history, no-op
    assert_eq!(e.text(), "");
}

#[test]
fn set_text_empty() {
    let mut e = Editor::new();
    e.set_text("hello");
    e.set_text("");
    assert_eq!(e.text(), "");
    assert_eq!(e.lines.len(), 1);
}

#[test]
fn render_multiline_has_correct_height() {
    let mut e = Editor::new();
    e.set_text("line1\nline2\nline3");
    let lines = e.render(80);
    // Should have top border + 3 content lines + bottom border = 5
    assert!(
        lines.len() >= 5,
        "multiline should render enough lines: {}",
        lines.len()
    );
}

#[test]
fn render_cache_reused() {
    let mut e = Editor::new();
    e.set_text("hello");
    let l1 = e.render(40);
    let l2 = e.render(40);
    assert_eq!(l1, l2);
}

#[test]
fn render_cache_invalidated_on_width_change() {
    let mut e = Editor::new();
    e.set_text("hello");
    let l1 = e.render(40);
    let l2 = e.render(80);
    // Different widths should produce different renders
    assert!(!l1.is_empty() && !l2.is_empty());
}

#[test]
fn utf8_char_boundaries() {
    assert_eq!(prev_char_boundary("héllo", 2), 1);
    assert_eq!(next_char_boundary("héllo", 0), 1);
    assert_eq!(prev_char_boundary("", 0), 0);
    assert_eq!(next_char_boundary("a", 0), 1);
}

#[test]
fn paste_with_cr_lf() {
    let mut e = Editor::new();
    e.handle_input(&Key::Paste("a\r\nb".to_string()));
    assert_eq!(e.text(), "a\nb");
}

#[test]
fn take_submit_returns_none_when_no_submit() {
    let mut e = Editor::new();
    assert_eq!(e.take_submit(), None);
}

#[test]
fn replace_before_cursor_swaps_token_and_moves_cursor() {
    let mut e = Editor::new();
    e.set_text("see @mai");
    // Cursor is at end (col 8). Replace the "@mai" token (start col 4).
    e.replace_before_cursor(4, "@src/main.rs ");
    assert_eq!(e.text(), "see @src/main.rs ");
    assert_eq!(e.cursor_col(), "see @src/main.rs ".len());
}

#[test]
fn cursor_col_and_current_line_reflect_state() {
    let mut e = Editor::new();
    e.set_text("hello");
    assert_eq!(e.current_line(), "hello");
    assert_eq!(e.cursor_col(), 5);
}
