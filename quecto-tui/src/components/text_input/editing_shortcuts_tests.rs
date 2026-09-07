use super::*;

#[test]
fn ctrl_w_kills_whitespace_delimited_chunk_and_yanks_it() {
    let mut e = Editor::new();
    e.set_text("open src/utils/foo.ts  ");
    e.handle_input(&Key::Ctrl('w'));
    assert_eq!(e.text(), "open ");
    e.handle_input(&Key::Ctrl('y'));
    assert_eq!(e.text(), "open src/utils/foo.ts  ");
    assert_eq!(e.cursor_col(), e.current_line().len());
}

#[test]
fn alt_d_uses_unicode_alphanumeric_words_and_stays_on_line() {
    let mut e = Editor::new();
    e.set_text("go --東京_42!\nnext");
    e.handle_input(&Key::Up);
    e.handle_input(&Key::Home);
    for _ in 0..2 {
        e.handle_input(&Key::Right);
    }
    e.handle_input(&Key::Alt('d'));
    assert_eq!(e.text(), "go_42!\nnext");
    e.handle_input(&Key::Ctrl('y'));
    assert_eq!(e.text(), "go --東京_42!\nnext");
}

#[test]
fn line_kills_fill_single_buffer_but_empty_kills_do_not_replace_it() {
    let mut e = Editor::new();
    e.set_text("αβ tail");
    e.handle_input(&Key::Ctrl('k')); // empty at end
    e.handle_input(&Key::Ctrl('u'));
    assert_eq!(e.text(), "");
    e.handle_input(&Key::Ctrl('k')); // must retain αβ tail
    e.handle_input(&Key::Ctrl('y'));
    assert_eq!(e.text(), "αβ tail");
}

#[test]
fn ordinary_deletes_do_not_replace_kill_buffer_and_yank_without_kill_is_noop() {
    let mut e = Editor::new();
    e.handle_input(&Key::Ctrl('y'));
    assert_eq!(e.text(), "");
    e.set_text("one two");
    e.handle_input(&Key::Ctrl('w'));
    e.handle_input(&Key::Backspace);
    e.handle_input(&Key::Home);
    e.handle_input(&Key::Delete);
    e.handle_input(&Key::Ctrl('y'));
    assert_eq!(e.text(), "twone");
}

#[test]
fn ctrl_arrows_are_exact_alt_word_navigation_aliases_across_lines() {
    let mut ctrl = Editor::new();
    let mut alt = Editor::new();
    ctrl.set_text("one two\nthree");
    alt.set_text("one two\nthree");
    for (ctrl_key, alt_key) in [
        (Key::CtrlLeft, Key::Alt('b')),
        (Key::CtrlLeft, Key::Alt('b')),
        (Key::CtrlRight, Key::Alt('f')),
    ] {
        ctrl.handle_input(&ctrl_key);
        alt.handle_input(&alt_key);
        assert_eq!(ctrl.text(), alt.text());
        assert_eq!(ctrl.cursor_col(), alt.cursor_col());
        assert_eq!(ctrl.current_line(), alt.current_line());
    }
}
