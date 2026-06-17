use super::*;

#[test]
fn parse_arrow_up() {
    let input = b"\x1b[A";
    let (key, n) = parse_key(input).unwrap();
    assert_eq!(key, Key::Up);
    assert_eq!(n, 3);
}

#[test]
fn parse_arrow_down() {
    let (key, _) = parse_key(b"\x1b[B").unwrap();
    assert_eq!(key, Key::Down);
}

#[test]
fn parse_arrow_right() {
    let (key, _) = parse_key(b"\x1b[C").unwrap();
    assert_eq!(key, Key::Right);
}

#[test]
fn parse_arrow_left() {
    let (key, _) = parse_key(b"\x1b[D").unwrap();
    assert_eq!(key, Key::Left);
}

#[test]
fn kitty_arrow_press() {
    let (key, n) = parse_key(b"\x1b[1;1u").unwrap();
    assert_eq!(key, Key::Up);
    assert_eq!(n, 6);
}

#[test]
fn kitty_arrow_release_is_not_actionable() {
    let (key, n) = parse_key(b"\x1b[1;1:3u").unwrap();
    assert_eq!(key, Key::Unknown(b"1;1:3".to_vec()));
    assert_eq!(n, 8);
}

#[test]
fn kitty_arrow_release_final_is_detected() {
    assert!(crate::interface::kitty::is_key_release(b"\x1b[1;1:3A"));
}

#[test]
fn parse_enter_cr() {
    let (key, n) = parse_key(b"\r").unwrap();
    assert_eq!(key, Key::Enter);
    assert_eq!(n, 1);
}

#[test]
fn parse_enter_lf() {
    let (key, _) = parse_key(b"\n").unwrap();
    assert_eq!(key, Key::Enter);
}

#[test]
fn parse_backspace() {
    let (key, _) = parse_key(b"\x7f").unwrap();
    assert_eq!(key, Key::Backspace);
}

#[test]
fn parse_escape_bare() {
    let (key, n) = parse_key(b"\x1b").unwrap();
    assert_eq!(key, Key::Escape);
    assert_eq!(n, 1);
}

#[test]
fn parse_tab() {
    let (key, _) = parse_key(b"\t").unwrap();
    assert_eq!(key, Key::Tab);
}

#[test]
fn parse_delete() {
    let (key, _) = parse_key(b"\x1b[3~").unwrap();
    assert_eq!(key, Key::Delete);
}

#[test]
fn parse_home() {
    let (key, _) = parse_key(b"\x1b[H").unwrap();
    assert_eq!(key, Key::Home);
}

#[test]
fn parse_end() {
    let (key, _) = parse_key(b"\x1b[F").unwrap();
    assert_eq!(key, Key::End);
}

#[test]
fn parse_ctrl_c() {
    let (key, _) = parse_key(b"\x03").unwrap();
    assert_eq!(key, Key::Ctrl('c'));
    assert!(key.is_ctrl_c());
}

#[test]
fn parse_ctrl_a() {
    let (key, _) = parse_key(b"\x01").unwrap();
    assert_eq!(key, Key::Ctrl('a'));
}

#[test]
fn parse_ctrl_d() {
    let (key, _) = parse_key(b"\x04").unwrap();
    assert_eq!(key, Key::Ctrl('d'));
}

#[test]
fn parse_printable_ascii() {
    let (key, n) = parse_key(b"a").unwrap();
    assert_eq!(key, Key::Char('a'));
    assert_eq!(n, 1);
}

#[test]
fn parse_printable_utf8() {
    let input = "é".as_bytes();
    let (key, n) = parse_key(input).unwrap();
    assert_eq!(key, Key::Char('é'));
    assert_eq!(n, 2);
}

#[test]
fn parse_printable_cjk() {
    let input = "日".as_bytes();
    let (key, n) = parse_key(input).unwrap();
    assert_eq!(key, Key::Char('日'));
    assert_eq!(n, 3);
}

#[test]
fn parse_alt_a() {
    let (key, n) = parse_key(b"\x1ba").unwrap();
    assert_eq!(key, Key::Alt('a'));
    assert_eq!(n, 2);
}

#[test]
fn parse_page_up() {
    let (key, _) = parse_key(b"\x1b[5~").unwrap();
    assert_eq!(key, Key::PageUp);
}

#[test]
fn parse_page_down() {
    let (key, _) = parse_key(b"\x1b[6~").unwrap();
    assert_eq!(key, Key::PageDown);
}

#[test]
fn parse_shift_tab() {
    let (key, _) = parse_key(b"\x1b[Z").unwrap();
    assert_eq!(key, Key::BackTab);
}

#[test]
fn parse_bracketed_paste() {
    let input = b"\x1b[200~hello world\x1b[201~";
    let (key, n) = parse_key(input).unwrap();
    assert_eq!(key, Key::Paste("hello world".to_string()));
    assert_eq!(n, input.len());
}

#[test]
fn parse_bracketed_paste_normalizes_crlf() {
    let input = b"\x1b[200~alpha\r\nbeta\r\ngamma\x1b[201~";
    let (key, n) = parse_key(input).unwrap();
    assert_eq!(key, Key::Paste("alpha\nbeta\ngamma".to_string()));
    assert_eq!(n, input.len());
}

#[test]
fn parse_empty_returns_none() {
    assert!(parse_key(b"").is_none());
}

#[test]
fn parse_incomplete_csi_returns_none() {
    // \x1b[ without a terminator — should return None (wait for more input)
    assert!(parse_key(b"\x1b[").is_none());
}

// ── Kitty protocol Ctrl+letter tests (issue #496) ─────────────────

#[test]
fn kitty_ctrl_d() {
    // CSI 100;5u — keycode 100='d', modifier 5=Ctrl+1
    let (key, _) = parse_key(b"\x1b[100;5u").unwrap();
    assert_eq!(key, Key::Ctrl('d'));
}

#[test]
fn kitty_ctrl_c() {
    // CSI 99;5u — keycode 99='c', modifier 5=Ctrl+1
    let (key, _) = parse_key(b"\x1b[99;5u").unwrap();
    assert_eq!(key, Key::Ctrl('c'));
}

#[test]
fn kitty_ctrl_a() {
    let (key, _) = parse_key(b"\x1b[97;5u").unwrap();
    assert_eq!(key, Key::Ctrl('a'));
}

#[test]
fn kitty_ctrl_z() {
    let (key, _) = parse_key(b"\x1b[122;5u").unwrap();
    assert_eq!(key, Key::Ctrl('z'));
}

#[test]
fn kitty_ctrl_l() {
    let (key, _) = parse_key(b"\x1b[108;5u").unwrap();
    assert_eq!(key, Key::Ctrl('l'));
}

#[test]
fn kitty_ctrl_o() {
    let (key, _) = parse_key(b"\x1b[111;5u").unwrap();
    assert_eq!(key, Key::Ctrl('o'));
}

#[test]
fn kitty_ctrl_shift_letters_accept_lowercase_keycodes() {
    let (key, _) = parse_key(b"\x1b[97;6u").unwrap();
    assert_eq!(key, Key::CtrlShift('a'));
    let (key, _) = parse_key(b"\x1b[110;6u").unwrap();
    assert_eq!(key, Key::CtrlShift('n'));
    let (key, _) = parse_key(b"\x1b[119;6u").unwrap();
    assert_eq!(key, Key::CtrlShift('w'));
}

#[test]
fn kitty_ctrl_shift_letters_accept_uppercase_shifted_keycodes() {
    let (key, _) = parse_key(b"\x1b[65;6u").unwrap();
    assert_eq!(key, Key::CtrlShift('a'));
    let (key, _) = parse_key(b"\x1b[78;6u").unwrap();
    assert_eq!(key, Key::CtrlShift('n'));
    let (key, _) = parse_key(b"\x1b[87;6u").unwrap();
    assert_eq!(key, Key::CtrlShift('w'));
}

#[test]
fn kitty_ctrl_shift_letters_accept_event_type_suffix() {
    let (key, _) = parse_key(b"\x1b[65;6:1u").unwrap();
    assert_eq!(key, Key::CtrlShift('a'));
    let (key, _) = parse_key(b"\x1b[78;6:1u").unwrap();
    assert_eq!(key, Key::CtrlShift('n'));
    let (key, _) = parse_key(b"\x1b[87;6:1u").unwrap();
    assert_eq!(key, Key::CtrlShift('w'));
}

#[test]
fn kitty_ctrl_shift_letters_accept_alternate_key_fields() {
    let (key, _) = parse_key(b"\x1b[65:65:97;6:1u").unwrap();
    assert_eq!(key, Key::CtrlShift('a'));
    let (key, _) = parse_key(b"\x1b[78:78:110;6:1u").unwrap();
    assert_eq!(key, Key::CtrlShift('n'));
    let (key, _) = parse_key(b"\x1b[87:87:119;6:1u").unwrap();
    assert_eq!(key, Key::CtrlShift('w'));
}

#[test]
fn kitty_ctrl_shift_uses_base_layout_for_non_latin_codepoints() {
    let (key, _) = parse_key(b"\x1b[1092::97;6:1u").unwrap();
    assert_eq!(key, Key::CtrlShift('a'));
}

#[test]
fn kitty_ctrl_shift_release_event_is_not_actionable() {
    let (key, _) = parse_key(b"\x1b[65;6:3u").unwrap();
    assert!(matches!(key, Key::Unknown(_)));
}

#[test]
fn modify_other_keys_ctrl_shift_letters() {
    let (key, _) = parse_key(b"\x1b[27;6;65~").unwrap();
    assert_eq!(key, Key::CtrlShift('a'));
    let (key, _) = parse_key(b"\x1b[27;6;78~").unwrap();
    assert_eq!(key, Key::CtrlShift('n'));
    let (key, _) = parse_key(b"\x1b[27;6;87~").unwrap();
    assert_eq!(key, Key::CtrlShift('w'));
}

#[test]
fn modify_other_keys_ignores_non_standard_two_field_tilde_variant() {
    let (key, _) = parse_key(b"\x1b[65;6~").unwrap();
    assert!(matches!(key, Key::Unknown(_)));
}

#[test]
fn kitty_plain_d_no_modifier() {
    // CSI 100;1u — keycode 100='d', modifier 1=none
    let (key, _) = parse_key(b"\x1b[100;1u").unwrap();
    assert_eq!(key, Key::Char('d'));
}

#[test]
fn kitty_alt_d() {
    // CSI 100;3u — keycode 100='d', modifier 3=Alt+1
    let (key, _) = parse_key(b"\x1b[100;3u").unwrap();
    assert_eq!(key, Key::Alt('d'));
}

#[test]
fn kitty_shift_enter_still_works() {
    let (key, _) = parse_key(b"\x1b[13;2u").unwrap();
    assert_eq!(key, Key::ShiftEnter);
}

#[test]
fn kitty_modifier_zero_no_panic() {
    // Modifier 0 is malformed but should not panic (saturating_sub).
    let (key, _) = parse_key(b"\x1b[100;0u").unwrap();
    // Should parse as plain 'd' (no modifiers after saturating_sub).
    assert_eq!(key, Key::Char('d'));
}

#[test]
fn kitty_ctrl_alt_d_treated_as_ctrl() {
    // Modifier 7 = Ctrl+Alt+1 → (7-1)=6, ctrl=true, alt=true.
    // Ctrl arm matches first (Alt dropped deliberately).
    let (key, _) = parse_key(b"\x1b[100;7u").unwrap();
    assert_eq!(key, Key::Ctrl('d'));
}

// ── SGR mouse scroll tests (issue #519) ───────────────────────────

#[test]
fn sgr_mouse_scroll_up() {
    // \x1b[<64;10;5M — scroll up at column 10, row 5 (11 bytes)
    let input = b"\x1b[<64;10;5M";
    assert_eq!(input.len(), 11);
    let (key, n) = parse_key(input).unwrap();
    assert_eq!(key, Key::ScrollUp);
    assert_eq!(n, 11);
}

#[test]
fn sgr_mouse_scroll_down() {
    // \x1b[<65;10;5M — scroll down at column 10, row 5 (11 bytes)
    let input = b"\x1b[<65;10;5M";
    assert_eq!(input.len(), 11);
    let (key, n) = parse_key(input).unwrap();
    assert_eq!(key, Key::ScrollDown);
    assert_eq!(n, 11);
}

#[test]
fn sgr_mouse_left_click_press() {
    // \x1b[<0;10;5M — left click press at col 10, row 5 → MousePress(9, 4) (0-indexed)
    let (key, _) = parse_key(b"\x1b[<0;10;5M").unwrap();
    assert_eq!(key, Key::MousePress(9, 4));
}

#[test]
fn sgr_mouse_scroll_up_release() {
    // \x1b[<64;10;5m — scroll up release (lowercase m)
    let (key, _) = parse_key(b"\x1b[<64;10;5m").unwrap();
    assert_eq!(key, Key::ScrollUp);
}

#[test]
fn sgr_mouse_incomplete_returns_none() {
    // Incomplete SGR mouse sequence
    assert!(parse_key(b"\x1b[<64;10;").is_none());
}

// ── Mouse press/drag/release tests (issue #528) ──────────────────

#[test]
fn sgr_mouse_left_release() {
    // \x1b[<0;20;10m — left release at col 20, row 10 → MouseRelease(19, 9)
    let (key, _) = parse_key(b"\x1b[<0;20;10m").unwrap();
    assert_eq!(key, Key::MouseRelease(19, 9));
}

#[test]
fn sgr_mouse_drag() {
    // \x1b[<32;15;7M — left drag at col 15, row 7 → MouseDrag(14, 6)
    let (key, _) = parse_key(b"\x1b[<32;15;7M").unwrap();
    assert_eq!(key, Key::MouseDrag(14, 6));
}

#[test]
fn sgr_mouse_press_at_origin() {
    // Column 1, row 1 → 0-indexed (0, 0)
    let (key, _) = parse_key(b"\x1b[<0;1;1M").unwrap();
    assert_eq!(key, Key::MousePress(0, 0));
}

#[test]
fn sgr_mouse_right_click_ignored() {
    // Button 2 = right click → Unknown
    let (key, _) = parse_key(b"\x1b[<2;10;5M").unwrap();
    assert!(matches!(key, Key::Unknown(_)));
}

#[test]
fn sgr_mouse_right_release_ignored() {
    // Button 2 release → Unknown (only left button release triggers MouseRelease)
    let (key, _) = parse_key(b"\x1b[<2;10;5m").unwrap();
    assert!(matches!(key, Key::Unknown(_)));
}
