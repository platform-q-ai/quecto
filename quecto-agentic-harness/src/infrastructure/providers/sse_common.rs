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

/// Render a `reqwest` transport error together with its source chain.
///
/// `reqwest::Error`'s `Display` only prints its top-level message
/// (e.g. `"error sending request for url (…)"`); the concrete cause —
/// `"connection refused"`, `"connection reset"`, `"dns error"`,
/// `"operation timed out"` — lives in `source()`. Dropping it loses the one
/// signal the retry classifier (`classify_provider_error`) needs to recognise a
/// transient `Network` failure, so a retryable connection blip is misclassified
/// as non-retryable `Unknown` and fails the turn without a single retry.
///
/// This walks the full `std::error::Error` source chain and appends each cause,
/// so the resulting string contains the underlying keyword (`connection`,
/// `timed out`, `dns`, …) the classifier matches on.
pub fn format_send_error(err: &reqwest::Error) -> String {
    use std::error::Error;

    let mut out = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        out.push_str(": ");
        out.push_str(&cause.to_string());
        source = cause.source();
    }
    out
}

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

/// Extract a `Retry-After` / `retry-after-ms` hint from HTTP response headers
/// and render it as a suffix to append to the error string, so the retry
/// decorator's `parse_retry_after` honours it on *real* provider responses
/// (#931). Without this the hint only ever existed in the JSON body of some
/// providers (often not at all), so the decorator always fell back to
/// exponential backoff. Returns an empty string when neither header is present.
///
/// Pass `response.headers()` *before* the response body is consumed (`.text()`
/// takes the response by value).
pub fn retry_after_suffix(headers: &reqwest::header::HeaderMap) -> String {
    if let Some(ms) = headers
        .get("retry-after-ms")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return format!(" retry-after-ms: {ms}");
    }
    if let Some(secs) = headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return format!(" retry-after: {secs}");
    }
    String::new()
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
#[path = "sse_common_tests.rs"]
mod tests;
#[cfg(test)]
mod pump_tests {
    use super::*;
    use crate::domain::message::LlmResponse;
    use std::sync::{Arc, Mutex};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[derive(Clone, Default)]
    struct RecordingHandler {
        lines: Arc<Mutex<Vec<String>>>,
        done_on: Option<String>,
    }

    impl SseHandler for RecordingHandler {
        async fn process_line(
            &mut self,
            line: &str,
            tx: &tokio::sync::mpsc::Sender<StreamEvent>,
        ) -> SseLineOutcome {
            self.lines.lock().unwrap().push(line.to_string());
            if self.done_on.as_deref() == Some(line) {
                let _ = tx
                    .send(StreamEvent::Done(LlmResponse {
                        content: Some("done".into()),
                        tool_calls: vec![],
                        usage: None,
                        stop_reason: None,
                        thinking_blocks: vec![],
                    }))
                    .await;
                SseLineOutcome::Done
            } else {
                SseLineOutcome::Continue
            }
        }

        async fn on_eof(&mut self, tx: &tokio::sync::mpsc::Sender<StreamEvent>) {
            let _ = tx.send(StreamEvent::TextDelta("eof".into())).await;
        }
    }

    #[tokio::test]
    async fn pump_sse_dispatches_lines_and_calls_eof() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sse"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"data: one\r\ndata: two\n"))
            .mount(&server)
            .await;
        let mut response = reqwest::get(format!("{}/sse", server.uri())).await.unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let lines = Arc::new(Mutex::new(Vec::new()));
        let mut handler = RecordingHandler {
            lines: Arc::clone(&lines),
            done_on: None,
        };

        pump_sse(&mut response, &tx, &mut handler).await;

        assert_eq!(
            *lines.lock().unwrap(),
            vec!["data: one".to_string(), "data: two".to_string()]
        );
        assert!(matches!(rx.recv().await, Some(StreamEvent::TextDelta(text)) if text == "eof"));
    }

    #[tokio::test]
    async fn pump_sse_stops_when_handler_returns_done() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sse"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"first\nstop\nlast\n"))
            .mount(&server)
            .await;
        let mut response = reqwest::get(format!("{}/sse", server.uri())).await.unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let lines = Arc::new(Mutex::new(Vec::new()));
        let mut handler = RecordingHandler {
            lines: Arc::clone(&lines),
            done_on: Some("stop".into()),
        };

        pump_sse(&mut response, &tx, &mut handler).await;

        assert_eq!(
            *lines.lock().unwrap(),
            vec!["first".to_string(), "stop".to_string()]
        );
        assert!(matches!(rx.recv().await, Some(StreamEvent::Done(_))));
    }

    #[tokio::test]
    async fn pump_sse_rejects_oversized_line_without_newline() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sse"))
            .respond_with(
                ResponseTemplate::new(200).set_body_bytes(vec![b'x'; MAX_SSE_LINE_BYTES + 1]),
            )
            .mount(&server)
            .await;
        let mut response = reqwest::get(format!("{}/sse", server.uri())).await.unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let mut handler = RecordingHandler::default();

        pump_sse(&mut response, &tx, &mut handler).await;

        assert!(
            matches!(rx.recv().await, Some(StreamEvent::Error(message)) if message.contains("exceeds"))
        );
    }

    #[tokio::test]
    async fn pump_sse_skips_invalid_utf8_line() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sse"))
            .respond_with(
                ResponseTemplate::new(200).set_body_bytes(vec![0xff, b'\n', b'o', b'k', b'\n']),
            )
            .mount(&server)
            .await;
        let mut response = reqwest::get(format!("{}/sse", server.uri())).await.unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        let lines = Arc::new(Mutex::new(Vec::new()));
        let mut handler = RecordingHandler {
            lines: Arc::clone(&lines),
            done_on: None,
        };

        pump_sse(&mut response, &tx, &mut handler).await;

        assert_eq!(*lines.lock().unwrap(), vec!["ok".to_string()]);
        assert!(matches!(rx.recv().await, Some(StreamEvent::TextDelta(text)) if text == "eof"));
    }
}

#[cfg(test)]
mod pump_w5_cov_tests {
    use super::*;
    use crate::domain::message::LlmResponse;
    use std::sync::{Arc, Mutex};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[derive(Clone, Default)]
    struct LinesHandler {
        lines: Arc<Mutex<Vec<String>>>,
    }

    impl SseHandler for LinesHandler {
        async fn process_line(
            &mut self,
            line: &str,
            _tx: &tokio::sync::mpsc::Sender<StreamEvent>,
        ) -> SseLineOutcome {
            self.lines
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(line.to_string());
            SseLineOutcome::Continue
        }

        async fn on_eof(&mut self, tx: &tokio::sync::mpsc::Sender<StreamEvent>) {
            let _ = tx
                .send(StreamEvent::Done(LlmResponse {
                    content: Some("eof".into()),
                    tool_calls: vec![],
                    usage: None,
                    stop_reason: None,
                    thinking_blocks: vec![],
                }))
                .await;
        }
    }

    #[tokio::test]
    async fn pump_sse_forwards_a_split_data_line_then_done() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/split"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"data: split\n"))
            .mount(&server)
            .await;
        let mut response = reqwest::get(format!("{}/split", server.uri()))
            .await
            .unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let lines = Arc::new(Mutex::new(Vec::new()));
        let mut handler = LinesHandler {
            lines: lines.clone(),
        };

        pump_sse(&mut response, &tx, &mut handler).await;

        assert_eq!(
            *lines.lock().expect("lines mutex is not poisoned"),
            vec!["data: split".to_string()]
        );
        assert!(matches!(rx.recv().await, Some(StreamEvent::Done(_))));
    }
}
