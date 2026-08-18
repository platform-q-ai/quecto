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
    assert!(crate::components::kitty::is_key_release(b"\x1b[1;1:3A"));
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

// ── Coverage: control bytes, SS3, CSI terminators ─────────────────────

#[test]
fn cov_ctrl_at_from_nul() {
    assert_eq!(parse_key(&[0x00]).unwrap().0, Key::Ctrl('@'));
}

#[test]
fn cov_ctrl_k_and_l() {
    assert_eq!(parse_key(&[0x0B]).unwrap().0, Key::Ctrl('k'));
    assert_eq!(parse_key(&[0x0C]).unwrap().0, Key::Ctrl('l'));
}

#[test]
fn cov_del_byte_is_backspace() {
    assert_eq!(parse_key(&[0x7F]).unwrap().0, Key::Backspace);
}

#[test]
fn cov_ss3_arrows_and_home_end() {
    assert_eq!(parse_key(b"\x1bOA").unwrap().0, Key::Up);
    assert_eq!(parse_key(b"\x1bOB").unwrap().0, Key::Down);
    assert_eq!(parse_key(b"\x1bOC").unwrap().0, Key::Right);
    assert_eq!(parse_key(b"\x1bOD").unwrap().0, Key::Left);
    assert_eq!(parse_key(b"\x1bOH").unwrap().0, Key::Home);
    assert_eq!(parse_key(b"\x1bOF").unwrap().0, Key::End);
}

#[test]
fn cov_ss3_unknown_and_incomplete() {
    assert!(matches!(parse_key(b"\x1bOZ").unwrap().0, Key::Unknown(_)));
    assert!(parse_key(b"\x1bO").is_none());
}

#[test]
fn cov_alt_enter_cr_and_lf() {
    assert_eq!(parse_key(b"\x1b\r").unwrap().0, Key::Alt('\n'));
    assert_eq!(parse_key(b"\x1b\n").unwrap().0, Key::Alt('\n'));
}

#[test]
fn cov_escape_followed_by_unhandled_byte() {
    let (key, n) = parse_key(b"\x1b\x01").unwrap();
    assert_eq!(key, Key::Escape);
    assert_eq!(n, 1);
}

#[test]
fn cov_csi_home_end_terminators() {
    assert_eq!(parse_key(b"\x1b[H").unwrap().0, Key::Home);
    assert_eq!(parse_key(b"\x1b[F").unwrap().0, Key::End);
}

#[test]
fn cov_csi_backtab_terminator() {
    assert_eq!(parse_key(b"\x1b[Z").unwrap().0, Key::BackTab);
}

#[test]
fn cov_csi_tilde_variants() {
    assert_eq!(parse_key(b"\x1b[1~").unwrap().0, Key::Home);
    assert_eq!(parse_key(b"\x1b[2~").unwrap().0, Key::Insert);
    assert_eq!(parse_key(b"\x1b[5~").unwrap().0, Key::PageUp);
    assert_eq!(parse_key(b"\x1b[6~").unwrap().0, Key::PageDown);
}

#[test]
fn cov_csi_unknown_terminator() {
    assert!(matches!(parse_key(b"\x1b[99X").unwrap().0, Key::Unknown(_)));
}

#[test]
fn cov_csi_unknown_tilde_param() {
    assert!(matches!(parse_key(b"\x1b[99~").unwrap().0, Key::Unknown(_)));
}

// ── Coverage: kitty keypad/special keys ───────────────────────────────

#[test]
fn cov_kitty_navigation_keycodes() {
    assert_eq!(parse_key(b"\x1b[1u").unwrap().0, Key::Up);
    assert_eq!(parse_key(b"\x1b[2u").unwrap().0, Key::Down);
    assert_eq!(parse_key(b"\x1b[3u").unwrap().0, Key::Right);
    assert_eq!(parse_key(b"\x1b[4u").unwrap().0, Key::Left);
    assert_eq!(parse_key(b"\x1b[5u").unwrap().0, Key::Home);
    assert_eq!(parse_key(b"\x1b[6u").unwrap().0, Key::End);
    assert_eq!(parse_key(b"\x1b[7u").unwrap().0, Key::PageUp);
    assert_eq!(parse_key(b"\x1b[8u").unwrap().0, Key::PageDown);
}

#[test]
fn cov_kitty_backspace_and_escape_keycodes() {
    assert_eq!(parse_key(b"\x1b[127u").unwrap().0, Key::Backspace);
    assert_eq!(parse_key(b"\x1b[27u").unwrap().0, Key::Escape);
}

#[test]
fn cov_kitty_alt_letter() {
    assert_eq!(parse_key(b"\x1b[97;3u").unwrap().0, Key::Alt('a'));
}

#[test]
fn cov_kitty_plain_uppercase_no_modifier() {
    assert_eq!(parse_key(b"\x1b[65;1u").unwrap().0, Key::Char('A'));
}

#[test]
fn cov_kitty_unknown_keycode() {
    assert!(matches!(
        parse_key(b"\x1b[200u").unwrap().0,
        Key::Unknown(_)
    ));
}

#[test]
fn cov_kitty_event_type_release_is_unknown() {
    // modifier field with event type ":3" (release) is not actionable.
    assert!(matches!(
        parse_key(b"\x1b[97;1:3u").unwrap().0,
        Key::Unknown(_)
    ));
}

// ── Coverage: SGR mouse edge cases ────────────────────────────────────

#[test]
fn cov_sgr_mouse_invalid_char_consumed() {
    let (key, n) = parse_key(b"\x1b[<0;x").unwrap();
    assert!(matches!(key, Key::Unknown(_)));
    assert_eq!(n, 6);
}

#[test]
fn cov_sgr_mouse_other_button_consumed() {
    let (key, _) = parse_key(b"\x1b[<2;5;5M").unwrap();
    assert!(matches!(key, Key::Unknown(_)));
}

// ── Coverage: modifyOtherKeys variants ────────────────────────────────

#[test]
fn cov_modify_other_keys_ctrl_only() {
    // CSI 27 ; 5 ; 65 ~  → Ctrl modifier (bit 2) on 'A' → Ctrl('a').
    assert_eq!(parse_key(b"\x1b[27;5;65~").unwrap().0, Key::Ctrl('a'));
}

#[test]
fn cov_modify_other_keys_alt_only() {
    // modifiers 3 → bit 1 (Alt) on 'a' → Alt('a').
    assert_eq!(parse_key(b"\x1b[27;3;97~").unwrap().0, Key::Alt('a'));
}

#[test]
fn cov_modify_other_keys_plain_printable() {
    assert_eq!(parse_key(b"\x1b[27;1;97~").unwrap().0, Key::Char('a'));
}

#[test]
fn modify_other_keys_tab_switch_chords_match_kitty() {
    // xterm modifyOtherKeys mode 2 must produce the same tab-cycle keys as
    // the kitty protocol (#1466 decision 5): codepoint 9 with Ctrl/Alt.
    assert_eq!(parse_key(b"\x1b[27;5;9~").unwrap().0, Key::TabSwitchNext);
    assert_eq!(parse_key(b"\x1b[27;3;9~").unwrap().0, Key::TabSwitchNext);
    assert_eq!(parse_key(b"\x1b[27;6;9~").unwrap().0, Key::TabSwitchPrev);
    assert_eq!(parse_key(b"\x1b[27;4;9~").unwrap().0, Key::TabSwitchPrev);
}

#[test]
fn modify_other_keys_ctrl_digit_aliases_alt_digit() {
    // Ctrl+9 and Alt+9 both alias the tab-focus primary.
    assert_eq!(parse_key(b"\x1b[27;5;57~").unwrap().0, Key::Alt('9'));
    assert_eq!(parse_key(b"\x1b[27;3;57~").unwrap().0, Key::Alt('9'));
}

// ── Coverage: utf8 fallback + convenience matchers ────────────────────

#[test]
fn cov_incomplete_utf8_returns_none() {
    // Lone UTF-8 continuation/start byte cannot form a char.
    assert!(parse_key(&[0xE2]).is_none());
}

#[test]
fn cov_is_ctrl_c_and_is_char() {
    assert!(Key::Ctrl('c').is_ctrl_c());
    assert!(!Key::Ctrl('d').is_ctrl_c());
    assert!(Key::Char('x').is_char());
    assert!(!Key::Enter.is_char());
}
