//! Shared filename sanitization for persistence modules.
//!
//! Both [`FileSessionStore`](super::session_store::FileSessionStore) and
//! [`FileContextSpillStore`](super::context_spill::FileContextSpillStore)
//! need to convert session keys into filesystem-safe names. This module
//! provides a single, collision-resistant implementation.

use std::fmt::Write;

/// Convert a session key to a filesystem-safe string.
///
/// - **Legacy-safe keys** (only `[A-Za-z0-9:_-.]`) are kept readable with
///   colons replaced by underscores.
/// - **All other keys** are hex-encoded with a `key_` prefix to avoid path
///   traversal, null bytes, and filename collisions.
///
/// The returned string does **not** include a file extension — callers
/// append `.json`, `.jsonl`, or use it as a directory name as needed.
pub fn sanitize_session_key(key: &str) -> String {
    if key.is_empty() {
        return hex_encode(key);
    }

    // Hex-encode dot-only keys (`.`, `..`, `...`, etc.) — they are
    // dangerous as directory/file names (traversal, current-dir, or
    // platform-specific weirdness like Windows stripping trailing dots).
    if key.chars().all(|c| c == '.') {
        return hex_encode(key);
    }

    // Hex-encode keys starting with a dot — they create hidden
    // files/directories on Unix, which may be missed by backups or `ls`.
    if key.starts_with('.') {
        return hex_encode(key);
    }

    if key.chars().all(is_legacy_safe_char) {
        return key.replace(':', "_");
    }

    hex_encode(key)
}

/// Hex-encode a key with a `key_` prefix for collision-resistant filenames.
fn hex_encode(key: &str) -> String {
    let mut encoded = String::with_capacity(key.len() * 2 + 4);
    encoded.push_str("key_");
    for b in key.as_bytes() {
        let _ = write!(encoded, "{b:02x}");
    }
    encoded
}

/// Characters considered safe for legacy-readable filenames.
fn is_legacy_safe_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, ':' | '_' | '-' | '.')
}

#[cfg(test)]
mod tests {
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
}
