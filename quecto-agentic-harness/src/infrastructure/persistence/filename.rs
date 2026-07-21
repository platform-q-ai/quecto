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
#[path = "filename_tests.rs"]
mod tests;
