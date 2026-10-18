use super::*;

#[test]
fn simple_alphanumeric_key() {
    assert_eq!(sanitize_session_key("simple"), "simple");
}

#[test]
fn colon_replaced_with_underscore() {
    assert_eq!(sanitize_session_key("telegram:12345"), "telegram_12345");
    assert_eq!(sanitize_session_key("cli:default"), "cli_default");
}

#[test]
fn dots_and_dashes_preserved() {
    assert_eq!(sanitize_session_key("my-session.v2"), "my-session.v2");
}

#[test]
fn empty_key_hex_encoded() {
    // Empty key produces `key_` (hex-encode of empty string).
    assert_eq!(sanitize_session_key(""), "key_");
}

#[test]
fn path_traversal_chars_hex_encoded() {
    let result = sanitize_session_key("../../tmp/escape");
    assert!(result.starts_with("key_"));
    assert!(!result.contains('/'));
    assert!(!result.contains('.'));
}

#[test]
fn null_bytes_hex_encoded() {
    let result = sanitize_session_key("a\0b");
    assert!(result.starts_with("key_"));
    assert!(!result.contains('\0'));
}

#[test]
fn slash_chars_hex_encoded() {
    let result = sanitize_session_key("a/b");
    assert!(result.starts_with("key_"));
}

#[test]
fn unsafe_keys_avoid_collisions() {
    let a = sanitize_session_key("a/b");
    let b = sanitize_session_key("a?b");
    assert_ne!(a, b);
}

#[test]
fn star_chars_hex_encoded() {
    let result = sanitize_session_key("a*b");
    assert!(result.starts_with("key_"));
}

#[test]
fn unicode_keys_hex_encoded() {
    let result = sanitize_session_key("日本語");
    assert!(result.starts_with("key_"));
}

#[test]
fn leading_dot_hex_encoded() {
    // Leading-dot keys create hidden files/dirs on Unix — hex-encode them.
    let result = sanitize_session_key(".hidden");
    assert!(result.starts_with("key_"));
    assert!(!result.starts_with('.'));
}

#[test]
fn dot_dot_hex_encoded() {
    // `..` is path traversal — must be hex-encoded for safety.
    let result = sanitize_session_key("..");
    assert!(result.starts_with("key_"));
    assert!(!result.contains(".."));
}

#[test]
fn single_dot_hex_encoded() {
    // `.` means "current directory" — hex-encode for safety.
    let result = sanitize_session_key(".");
    assert!(result.starts_with("key_"));
}

/// #2192 review: keys the readable form names alike get distinct digest
/// names, each of fixed length and hex only, the same for the same key.
#[test]
fn digest_names_keys_that_sanitize_alike_apart() {
    assert_eq!(
        sanitize_session_key("telegram:123"),
        sanitize_session_key("telegram_123")
    );
    let (colon, underscore) = (
        digest_session_key("telegram:123"),
        digest_session_key("telegram_123"),
    );
    assert_ne!(colon, underscore);
    assert_eq!(colon, digest_session_key("telegram:123"), "deterministic");
    for name in [colon, underscore, digest_session_key(&"k".repeat(10_000))] {
        assert_eq!(name.len(), DIGEST_NAME_LEN);
        assert!(
            name.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
            "{name}"
        );
    }
}
