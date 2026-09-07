use super::*;

fn press(editor: &mut Editor, keys: impl IntoIterator<Item = Key>) {
    for key in keys {
        editor.handle_input(&key);
    }
}

#[test]
fn ctrl_w_uses_unicode_whitespace_and_never_crosses_logical_lines() {
    let mut e = Editor::new();
    e.set_text("keep\nαβ\u{2003}src/工具.rs  ");
    e.handle_input(&Key::Ctrl('w'));
    assert_eq!(e.text(), "keep\nαβ\u{2003}");
    e.handle_input(&Key::Ctrl('y'));
    assert_eq!(e.text(), "keep\nαβ\u{2003}src/工具.rs  ");

    e.handle_input(&Key::Home);
    e.handle_input(&Key::Ctrl('w'));
    assert_eq!(e.text(), "keep\nαβ\u{2003}src/工具.rs  ");
}

#[test]
fn ctrl_w_deletes_from_inside_a_non_whitespace_chunk() {
    let mut e = Editor::new();
    e.set_text("one αβγ tail");
    press(
        &mut e,
        [
            Key::Home,
            Key::Right,
            Key::Right,
            Key::Right,
            Key::Right,
            Key::Right,
            Key::Right,
        ],
    );
    e.handle_input(&Key::Ctrl('w'));
    assert_eq!(e.text(), "one γ tail");
    e.handle_input(&Key::Ctrl('y'));
    assert_eq!(e.text(), "one αβγ tail");
}

#[test]
fn alt_d_uses_unicode_scalar_alphanumeric_runs_and_stays_on_line() {
    let mut e = Editor::new();
    e.set_text("go --東京_42!\nnext");
    press(&mut e, [Key::Up, Key::Home, Key::Right, Key::Right]);
    e.handle_input(&Key::Alt('d'));
    assert_eq!(e.text(), "go_42!\nnext");
    e.handle_input(&Key::Ctrl('y'));
    assert_eq!(e.text(), "go --東京_42!\nnext");
}

#[test]
fn alt_d_handles_inside_word_separators_combining_marks_and_no_candidate() {
    let mut e = Editor::new();
    e.set_text("écl😀e\u{301}x");
    press(&mut e, [Key::Home, Key::Right]);
    e.handle_input(&Key::Alt('d'));
    assert_eq!(e.text(), "é😀e\u{301}x");
    e.handle_input(&Key::Alt('d'));
    assert_eq!(e.text(), "é\u{301}x");
    e.handle_input(&Key::End);
    e.handle_input(&Key::Alt('d'));
    e.handle_input(&Key::Ctrl('y'));
    assert_eq!(e.text(), "é\u{301}x😀e");
}

#[test]
fn every_kill_producer_populates_the_single_yank_buffer() {
    let cases = [
        ("left right", vec![Key::Ctrl('u')]),
        ("left right", vec![Key::Home, Key::Ctrl('k')]),
        ("left right", vec![Key::Ctrl('w')]),
        ("left right", vec![Key::Home, Key::Alt('d')]),
    ];
    for (original, keys) in cases {
        let mut e = Editor::new();
        e.set_text(original);
        press(&mut e, keys);
        e.handle_input(&Key::Ctrl('y'));
        assert_eq!(e.text(), original);
    }
}

#[test]
fn empty_kills_preserve_the_previous_buffer() {
    let mut e = Editor::new();
    e.set_text("saved");
    e.handle_input(&Key::Ctrl('u'));
    for key in [
        Key::Ctrl('u'),
        Key::Ctrl('k'),
        Key::Ctrl('w'),
        Key::Alt('d'),
    ] {
        e.handle_input(&key);
    }
    e.handle_input(&Key::Ctrl('y'));
    assert_eq!(e.text(), "saved");
}

#[test]
fn ordinary_edits_do_not_replace_buffer_and_yank_repeats_at_cursor() {
    let mut e = Editor::new();
    e.handle_input(&Key::Ctrl('y'));
    assert_eq!(e.text(), "");
    e.set_text("one two");
    e.handle_input(&Key::Ctrl('w'));
    press(
        &mut e,
        [Key::Backspace, Key::Home, Key::Delete, Key::Char('X')],
    );
    e.handle_input(&Key::Ctrl('y'));
    e.handle_input(&Key::Ctrl('y'));
    assert_eq!(e.text(), "Xtwotwone");
    assert_eq!(e.cursor_col(), "Xtwotwo".len());
}

#[test]
fn kill_buffer_persists_for_the_editor_instance_across_draft_replacement() {
    let mut e = Editor::new();
    e.set_text("saved");
    e.handle_input(&Key::Ctrl('u'));
    e.set_text("new ");
    e.handle_input(&Key::Ctrl('y'));
    assert_eq!(e.text(), "new saved");
}

#[test]
fn ctrl_arrows_match_alt_navigation_step_for_step_at_boundaries_and_unicode() {
    for text in ["one two\nthree", "α β\n\nγ", ""] {
        let mut ctrl = Editor::new();
        let mut alt = Editor::new();
        ctrl.set_text(text);
        alt.set_text(text);
        for (ctrl_key, alt_key) in [
            (Key::CtrlLeft, Key::Alt('b')),
            (Key::CtrlLeft, Key::Alt('b')),
            (Key::CtrlRight, Key::Alt('f')),
            (Key::CtrlRight, Key::Alt('f')),
            (Key::CtrlRight, Key::Alt('f')),
        ] {
            ctrl.handle_input(&ctrl_key);
            alt.handle_input(&alt_key);
            assert_eq!(ctrl.text(), alt.text());
            assert_eq!(ctrl.cursor_col(), alt.cursor_col());
            assert_eq!(ctrl.current_line(), alt.current_line());
        }
    }
}
