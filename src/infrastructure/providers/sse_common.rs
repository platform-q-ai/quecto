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

use crate::domain::provider::StreamEvent;

/// Outcome of processing a single SSE line.
///
/// Returned by the per-line callback to signal whether the pump should
/// continue reading or terminate (because a terminal event was received).
pub enum SseLineOutcome {
    /// Continue processing the next line.
    Continue,
    /// The stream is complete — the handler has already sent `StreamEvent::Done`.
    Done,
}

/// Handler trait for processing SSE lines in the shared pump loop.
///
/// Each provider implements this to define how individual SSE lines are
/// interpreted and how the final response is assembled on EOF.
///
/// Uses native `async fn` in traits (RPITIT, stable since Rust 1.75) to
/// avoid per-line `Box::pin` heap allocations on the streaming hot path.
pub trait SseHandler: Send {
    /// Process one complete SSE line (with trailing `\n`/`\r` already stripped,
    /// per the SSE spec — lines are right-trimmed only, not fully trimmed).
    ///
    /// Return [`SseLineOutcome::Done`] when the stream should terminate.
    /// The handler must send `StreamEvent::Done` itself before returning `Done`.
    fn process_line(
        &mut self,
        line: &str,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> impl std::future::Future<Output = SseLineOutcome> + Send;

    /// Called when the response body is fully consumed without `process_line`
    /// ever returning `Done`. The handler should send `StreamEvent::Done`
    /// with whatever state it has accumulated.
    fn on_eof(
        &mut self,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> impl std::future::Future<Output = ()> + Send;
}

/// Read an SSE byte stream from `response`, split into newline-terminated
/// lines, and dispatch each to `handler`.
///
/// This is the shared pump loop used by all three providers (OpenAI, Codex,
/// Anthropic). It handles:
/// - Chunked byte accumulation with a carry buffer
/// - Guard against unbounded line growth ([`MAX_SSE_LINE_BYTES`])
/// - UTF-8 decoding of complete lines (skipping malformed lines)
/// - Stream read errors (emitted as `StreamEvent::Error`)
/// - Clean EOF (delegates to `handler.on_eof()`)
///
/// Lines are right-trimmed only (`\n`, `\r`), not fully trimmed, per the
/// SSE specification. Providers that previously used `.trim()` are unaffected
/// in practice since well-formed SSE servers do not emit leading whitespace.
pub async fn pump_sse<H: SseHandler>(
    response: &mut reqwest::Response,
    tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    handler: &mut H,
) {
    let mut carry: Vec<u8> = Vec::new();

    loop {
        let bytes = match response.chunk().await {
            Ok(Some(b)) => b,
            Ok(None) => break,
            Err(e) => {
                let _ = tx
                    .send(StreamEvent::Error(format!("stream read error: {e}")))
                    .await;
                return;
            }
        };

        // Guard against unbounded line growth from a misbehaving server.
        if carry.len() + bytes.len() > MAX_SSE_LINE_BYTES && !carry.contains(&b'\n') {
            let _ = tx
                .send(StreamEvent::Error("SSE line exceeds 1 MiB limit".into()))
                .await;
            return;
        }
        carry.extend_from_slice(&bytes);

        // Drain complete lines — decode in-place to avoid per-line allocation.
        while let Some(pos) = carry.iter().position(|&b| b == b'\n') {
            let done = if let Ok(line) = std::str::from_utf8(&carry[..=pos]) {
                let line = line.trim_end_matches(['\n', '\r']);
                matches!(handler.process_line(line, tx).await, SseLineOutcome::Done)
            } else {
                false
            };
            carry.drain(..=pos);
            if done {
                return;
            }
        }
    }

    // Clean EOF — let the handler finalize.
    handler.on_eof(tx).await;
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
