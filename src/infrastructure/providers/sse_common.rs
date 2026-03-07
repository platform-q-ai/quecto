//! Shared constants for SSE (Server-Sent Events) stream processing.
//!
//! Used by OpenAI, Codex, and Anthropic SSE pump implementations to avoid
//! duplicating buffer limits and error body truncation logic.

/// Maximum SSE line buffer size before rejecting a misbehaving server.
///
/// 1 MiB is generous for any well-formed SSE event. A server that emits
/// a single line exceeding this is almost certainly broken or adversarial.
pub const MAX_SSE_LINE_BYTES: usize = 1024 * 1024; // 1 MiB

/// Maximum size for HTTP error response bodies.
///
/// Caps error text included in `StreamEvent::Error` messages to prevent
/// unbounded memory usage on malformed or oversized error responses.
pub const MAX_ERROR_BODY_BYTES: usize = 4096;

/// Truncate an HTTP error body to [`MAX_ERROR_BODY_BYTES`].
///
/// Returns the body unchanged if it fits. Appends `"... (truncated)"`
/// if the body exceeds the limit. Truncation is UTF-8-safe — it finds
/// the nearest char boundary at or before the limit to avoid panicking
/// on multi-byte codepoints.
pub fn truncate_error_body(mut body: String) -> String {
    if body.len() > MAX_ERROR_BODY_BYTES {
        // Find the nearest char boundary at or before the limit.
        let mut end = MAX_ERROR_BODY_BYTES;
        while !body.is_char_boundary(end) && end > 0 {
            end -= 1;
        }
        body.truncate(end);
        body.push_str("... (truncated)");
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_sse_line_bytes_is_one_mib() {
        assert_eq!(MAX_SSE_LINE_BYTES, 1024 * 1024);
    }

    #[test]
    fn max_error_body_bytes_is_4096() {
        assert_eq!(MAX_ERROR_BODY_BYTES, 4096);
    }

    #[test]
    fn truncate_error_body_short_unchanged() {
        let body = "short error".to_string();
        assert_eq!(truncate_error_body(body), "short error");
    }

    #[test]
    fn truncate_error_body_exact_limit_unchanged() {
        let body = "x".repeat(MAX_ERROR_BODY_BYTES);
        let result = truncate_error_body(body.clone());
        assert_eq!(result, body);
    }

    #[test]
    fn truncate_error_body_over_limit_truncated() {
        let body = "x".repeat(MAX_ERROR_BODY_BYTES + 100);
        let result = truncate_error_body(body);
        assert!(result.ends_with("... (truncated)"));
        assert!(result.len() < MAX_ERROR_BODY_BYTES + 20);
    }

    #[test]
    fn truncate_error_body_utf8_safe() {
        // Build a string where byte 4096 falls inside a multi-byte char.
        // '€' is 3 bytes (E2 82 AC). Fill up to just before 4096, then add '€'
        // so the 3-byte char straddles the boundary.
        let padding = "x".repeat(MAX_ERROR_BODY_BYTES - 1); // 4095 bytes
        let body = format!("{padding}€€€"); // 4095 + 9 = 4104 bytes
        let result = truncate_error_body(body);
        // Must not panic, and must end with the truncation marker
        assert!(result.ends_with("... (truncated)"));
        // The result should be valid UTF-8 (it is, since it's a String)
        assert!(result.is_char_boundary(result.len()));
    }
}
