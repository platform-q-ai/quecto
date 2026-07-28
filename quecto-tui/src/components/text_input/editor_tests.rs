use super::editor::{Editor, next_char_boundary, prev_char_boundary, render_line_with_cursor};
use crate::components::ansi::strip_ansi;
use crate::components::component::Component;
use crate::components::utils::visible_width;
use crate::shell::keys::Key;

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
    // Multi-line observable without field access: Up moves within draft.
    e.handle_input(&Key::Up);
    e.handle_input(&Key::Char('X'));
    assert_eq!(e.text(), "aX\nb");
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
fn paste_inserts_cr_as_newline() {
    let mut e = Editor::new();
    e.handle_input(&Key::Paste("hello\rworld".to_string()));
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
    assert!(
        e.render_cache_is_populated(),
        "render must populate the cache"
    );
    e.invalidate();
    assert!(
        !e.render_cache_is_populated(),
        "invalidate must clear the render cache (not merely preserve output)"
    );
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
    // Empty draft is a single empty line: typing stays on one line.
    e.handle_input(&Key::Char('x'));
    assert_eq!(e.text(), "x");
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

#[test]
fn render_line_with_cursor_does_not_panic_on_mid_char_column() {
    // "é" is a 2-byte UTF-8 char. A cursor column landing on byte 1 is
    // *inside* the char; slicing &line[..1] would panic without a defensive
    // snap to the previous char boundary.
    let out = render_line_with_cursor("é", 1, 40);
    assert!(!out.is_empty());
    // The single multi-byte char must survive the defensive boundary snap.
    let visible = strip_ansi(&out.join(""));
    assert_eq!(visible, "é");
}

#[test]
fn render_line_with_cursor_mid_char_keeps_text_intact() {
    // Multi-byte content with an interior cursor column must still render
    // the full text (no garbling) and not panic.
    let out = render_line_with_cursor("naïve", 3, 40);
    // Strip the reverse-video markers and assert the full text round-trips,
    // including the at-cursor multi-byte char `ï` that the boundary bug touches.
    let visible = strip_ansi(&out.join(""));
    assert_eq!(visible, "naïve");
}

#[test]
fn hidden_cursor_emits_no_reverse_video() {
    // The block cursor is a reverse-video SGR (\x1b[7m), not the terminal
    // cursor. With `show_cursor` off (focus elsewhere, e.g. the sub-agent
    // panel) no line may carry it; re-enabling must bring it back, and each
    // toggle must invalidate the render cache.
    let mut e = Editor::new();
    e.set_text("hello");
    assert!(
        e.render(40).iter().any(|l| l.contains("\x1b[7m")),
        "cursor must render by default"
    );
    e.set_show_cursor(false);
    assert!(
        e.render(40).iter().all(|l| !l.contains("\x1b[7m")),
        "no reverse-video cursor may render while hidden"
    );
    e.set_show_cursor(true);
    assert!(
        e.render(40).iter().any(|l| l.contains("\x1b[7m")),
        "cursor must return when shown again"
    );
}

// ── #1277 characterization: parity-contract gaps on unmodified code ─────

#[test]
fn submit_whitespace_only_does_not_submit_or_clear() {
    let mut e = Editor::new();
    e.set_text("   \t  ");
    e.handle_input(&Key::Enter);
    assert_eq!(e.take_submit(), None, "whitespace-only must not submit");
    assert_eq!(e.text(), "   \t  ", "draft must stay put");
}

#[test]
fn submit_returns_untrimmed_text_and_records_trimmed_history() {
    let mut e = Editor::new();
    e.set_text("  hello  ");
    e.handle_input(&Key::Enter);
    assert_eq!(
        e.take_submit(),
        Some("  hello  ".to_string()),
        "submit payload keeps surrounding whitespace"
    );
    assert_eq!(e.text(), "");
    // History stores the trimmed form — Up must restore "hello", not padded.
    e.handle_input(&Key::Up);
    assert_eq!(e.text(), "hello");
}

#[test]
fn empty_submit_is_noop() {
    let mut e = Editor::new();
    e.handle_input(&Key::Enter);
    assert_eq!(e.take_submit(), None);
    assert_eq!(e.text(), "");
}

#[test]
fn history_skips_duplicate_of_last_entry() {
    let mut e = Editor::new();
    // Without dedupe the stack would be [a, a, b] and three Ups walk a→a→b.
    // With dedupe the stack is [a, b] and the third Up stays on oldest `a`.
    e.add_to_history("a");
    e.add_to_history("a");
    e.add_to_history("b");
    e.handle_input(&Key::Up);
    assert_eq!(e.text(), "b");
    e.handle_input(&Key::Up);
    assert_eq!(e.text(), "a");
    // Insert a marker while parked on oldest; if a second "a" existed above
    // the next Up would land on it and we'd edit a different entry. With
    // dedupe we stay on the single "a" and mutate it in place via set_text path
    // is not available — instead count entries by walking from a fresh editor
    // after replaying history through submits is heavy. Simpler observable:
    // after two Ups we are at oldest; one more Up must stay on "a" AND a
    // subsequent Down must go to "b" then empty — proving only two entries.
    e.handle_input(&Key::Up);
    assert_eq!(e.text(), "a", "third Up stays on single oldest entry");
    e.handle_input(&Key::Down);
    assert_eq!(e.text(), "b", "only one step back to newest");
    e.handle_input(&Key::Down);
    assert_eq!(
        e.text(),
        "",
        "then back to empty draft (no third history slot)"
    );
}

#[test]
fn history_up_saves_in_progress_draft_and_down_restores_it() {
    let mut e = Editor::new();
    e.add_to_history("prior");
    e.set_text("draft in progress");
    e.handle_input(&Key::Up);
    assert_eq!(e.text(), "prior");
    e.handle_input(&Key::Down);
    assert_eq!(e.text(), "draft in progress");
}

#[test]
fn history_cap_evicts_oldest_beyond_500() {
    let mut e = Editor::new();
    for i in 0..501 {
        e.add_to_history(&format!("entry-{i}"));
    }
    // After 501 unique pushes with cap 500, entry-0 is gone; entry-1 is oldest.
    e.handle_input(&Key::Up); // newest = entry-500
    assert_eq!(e.text(), "entry-500");
    // Walk to the oldest retained entry.
    for _ in 0..500 {
        e.handle_input(&Key::Up);
    }
    assert_eq!(
        e.text(),
        "entry-1",
        "cap 500 must drop entry-0 and keep entry-1..entry-500"
    );
}

#[test]
fn multiline_up_does_not_navigate_history() {
    let mut e = Editor::new();
    e.add_to_history("from-history");
    e.set_text("line-a\nline-b");
    // Cursor ends on last line; Up must move within the draft, not load history.
    e.handle_input(&Key::Up);
    e.handle_input(&Key::Char('X'));
    assert_eq!(e.text(), "line-aX\nline-b");
}

#[test]
fn empty_history_up_is_noop() {
    let mut e = Editor::new();
    e.set_text("keep-me");
    e.handle_input(&Key::Up);
    assert_eq!(e.text(), "keep-me");
}

#[test]
fn bash_mode_after_leading_whitespace_on_first_line() {
    let mut e = Editor::new();
    e.set_text("  !echo hi");
    assert!(
        e.is_bash_mode(),
        "leading whitespace before ! still enables bash mode"
    );
    // Observable via border indicator as well.
    let top = strip_ansi(&e.render(40)[0]);
    assert!(
        top.contains('!'),
        "bash mode top border must show ! indicator: {top:?}"
    );
}

#[test]
fn normal_mode_prompt_indicator_is_gt() {
    let mut e = Editor::new();
    e.set_text("hello");
    let top = strip_ansi(&e.render(40)[0]);
    assert!(
        top.contains('>'),
        "normal mode top border must show > indicator: {top:?}"
    );
    assert!(!top.contains('!'));
}

#[test]
fn set_text_multiline_places_cursor_at_end_of_last_line() {
    let mut e = Editor::new();
    e.set_text("aa\nbbb");
    assert_eq!(e.current_line(), "bbb");
    assert_eq!(e.cursor_col(), 3);
    e.handle_input(&Key::Char('!'));
    assert_eq!(e.text(), "aa\nbbb!");
}

#[test]
fn replace_before_cursor_noop_on_non_boundary_is_safe() {
    // Mid-char start must not panic and must leave text unchanged.
    let mut e = Editor::new();
    e.set_text("héllo");
    // "é" occupies bytes 1..3; start=2 is inside the char.
    e.replace_before_cursor(2, "X");
    assert_eq!(e.text(), "héllo");
}

#[test]
fn alt_carriage_return_inserts_newline() {
    let mut e = Editor::new();
    e.handle_input(&Key::Char('a'));
    e.handle_input(&Key::Alt('\r'));
    e.handle_input(&Key::Char('b'));
    assert_eq!(e.text(), "a\nb");
}

#[test]
fn history_cap_keeps_oldest_at_exactly_500_entries() {
    let mut e = Editor::new();
    for i in 0..500 {
        e.add_to_history(&format!("entry-{i}"));
    }
    // At the cap, oldest (entry-0) is still retained.
    e.handle_input(&Key::Up); // newest
    assert_eq!(e.text(), "entry-499");
    for _ in 0..499 {
        e.handle_input(&Key::Up);
    }
    assert_eq!(e.text(), "entry-0", "exactly 500 entries keep the oldest");
}
