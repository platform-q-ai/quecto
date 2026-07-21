use super::*;

#[test]
fn parse_response_valid() {
    let input = b"\x1b[?1u";
    assert_eq!(KittyProtocol::parse_response(input), Some(1));
}

#[test]
fn parse_response_flags_7() {
    let input = b"\x1b[?7u";
    assert_eq!(KittyProtocol::parse_response(input), Some(7));
}

#[test]
fn parse_response_no_match() {
    let input = b"hello world";
    assert_eq!(KittyProtocol::parse_response(input), None);
}

#[test]
fn parse_response_partial() {
    let input = b"\x1b[?";
    assert_eq!(KittyProtocol::parse_response(input), None);
}

#[test]
fn is_key_release_true() {
    assert!(is_key_release(b"\x1b[97;1:3u")); // 'a' release
}

#[test]
fn is_key_release_false() {
    assert!(!is_key_release(b"\x1b[97;1:1u")); // 'a' press
    assert!(!is_key_release(b"\x1b[A")); // arrow up (not Kitty)
}

#[test]
fn is_key_release_ignores_unanchored_text_containing_marker() {
    assert!(!is_key_release(b"typed text :3u"));
}

#[test]
fn is_key_release_ignores_alternate_key_fields() {
    assert!(!is_key_release(b"\x1b[65:65:97;6:1u"));
}

#[test]
fn new_starts_inactive() {
    let k = KittyProtocol::new();
    assert!(!k.active);
    assert!(!k.modify_other_keys);
}

#[test]
fn default_same_as_new() {
    let k = KittyProtocol::default();
    assert!(!k.active);
    assert!(!k.modify_other_keys);
}

#[test]
fn enable_sets_active() {
    let mut k = KittyProtocol::new();
    k.enable();
    assert!(k.active);
}

#[test]
fn disable_clears_active() {
    let mut k = KittyProtocol::new();
    k.enable();
    k.disable();
    assert!(!k.active);
}

#[test]
fn disable_when_inactive_is_noop() {
    let mut k = KittyProtocol::new();
    k.disable(); // should not panic
    assert!(!k.active);
}

#[test]
fn enable_modify_other_keys() {
    let mut k = KittyProtocol::new();
    k.enable_modify_other_keys();
    assert!(k.modify_other_keys);
}

#[test]
fn disable_modify_other_keys() {
    let mut k = KittyProtocol::new();
    k.enable_modify_other_keys();
    k.disable_modify_other_keys();
    assert!(!k.modify_other_keys);
}

#[test]
fn disable_modify_other_keys_when_inactive_is_noop() {
    let mut k = KittyProtocol::new();
    k.disable_modify_other_keys(); // should not panic
    assert!(!k.modify_other_keys);
}

#[test]
fn cleanup_clears_both() {
    let mut k = KittyProtocol::new();
    k.enable();
    k.enable_modify_other_keys();
    k.cleanup();
    assert!(!k.active);
    assert!(!k.modify_other_keys);
}

#[test]
fn parse_response_with_prefix_noise() {
    // Response embedded in other input
    let input = b"some noise\x1b[?15u";
    assert_eq!(KittyProtocol::parse_response(input), Some(15));
}

#[test]
fn parse_response_invalid_utf8() {
    let input = &[0xFF, 0xFE, 0x1b, b'[', b'?', b'1', b'u'];
    assert_eq!(KittyProtocol::parse_response(input), None);
}

#[test]
fn is_key_release_tilde_variant() {
    assert!(is_key_release(b"\x1b[5;1:3~")); // PageUp release
}

#[test]
fn is_key_release_invalid_utf8() {
    assert!(!is_key_release(&[0xFF, 0xFE]));
}

#[test]
fn query_does_not_panic() {
    let k = KittyProtocol::new();
    k.query(); // writes to stdout — just verify no panic
}
