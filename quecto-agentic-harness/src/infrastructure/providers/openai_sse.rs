//! Incremental SSE byte-stream parser for OpenAI chat completions.
//!
//! Extracted from `openai.rs` to keep both files under the 750-line limit.
//! Uses the shared SSE pump from [`sse_common`].

use crate::domain::message::{LlmResponse, ToolCall, UsageInfo};
use crate::domain::provider::StreamEvent;
use crate::infrastructure::providers::sse_common::{SseHandler, SseLineOutcome, pump_sse};

use super::OpenAiProvider;

/// SSE line handler for OpenAI chat completions.
struct OpenAiSseHandler {
    content: String,
    tool_calls: Vec<ToolCall>,
    usage: Option<UsageInfo>,
    /// Reused sink for `apply_delta`'s content extraction (which we ignore
    /// here, since content is accumulated into `content` directly).
    delta_scratch: String,
}

impl OpenAiSseHandler {
    fn new() -> Self {
        Self {
            content: String::new(),
            tool_calls: Vec::new(),
            usage: None,
            delta_scratch: String::new(),
        }
    }

    fn take_response(&mut self) -> LlmResponse {
        let content = if self.content.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.content))
        };
        LlmResponse {
            content,
            tool_calls: std::mem::take(&mut self.tool_calls),
            usage: self.usage.take(),
            stop_reason: None,
            thinking_blocks: vec![],
        }
    }
}

impl SseHandler for OpenAiSseHandler {
    async fn process_line(
        &mut self,
        line: &str,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> SseLineOutcome {
        let Some(data) = line.strip_prefix("data: ") else {
            return SseLineOutcome::Continue;
        };
        if data == "[DONE]" {
            let _ = tx.send(StreamEvent::Done(self.take_response())).await;
            return SseLineOutcome::Done;
        }
        if let Ok(chunk) = serde_json::from_str::<serde_json::Value>(data) {
            // Final usage chunk (requested via stream_options.include_usage).
            // Emitted with an empty `choices` array and a populated `usage`.
            if let Some(usage) = chunk.get("usage").and_then(|u| u.as_object()) {
                self.usage = Some(crate::infrastructure::providers::usage::parse_openai_usage(
                    usage,
                ));
            }
            if let Some(choices) = chunk.get("choices").and_then(|v| v.as_array()) {
                // Content is accumulated directly into `self.content` above, so
                // `apply_delta` only needs a throwaway sink for its own content
                // extraction. Reuse one buffer across choices instead of
                // allocating a fresh `String` per delta.
                for choice in choices {
                    let delta = choice.get("delta").unwrap_or(&serde_json::Value::Null);
                    if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
                        self.content.push_str(text);
                        let _ = tx.send(StreamEvent::TextDelta(text.to_string())).await;
                    }
                    self.delta_scratch.clear();
                    OpenAiProvider::apply_delta(
                        delta,
                        &mut self.delta_scratch,
                        &mut self.tool_calls,
                    );
                }
            }
        }
        SseLineOutcome::Continue
    }

    async fn on_eof(&mut self, tx: &tokio::sync::mpsc::Sender<StreamEvent>) {
        let _ = tx.send(StreamEvent::Done(self.take_response())).await;
    }
}

/// Consume an OpenAI SSE byte stream, emitting `StreamEvent`s per delta.
pub(crate) async fn pump_sse_bytes(
    response: &mut reqwest::Response,
    tx: &tokio::sync::mpsc::Sender<StreamEvent>,
) {
    let mut handler = OpenAiSseHandler::new();
    pump_sse(response, tx, &mut handler).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn handler_emits_text_delta_and_done_response() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let mut handler = OpenAiSseHandler::new();

        let outcome = handler
            .process_line(r#"data: {"choices":[{"delta":{"content":"hello"}}]}"#, &tx)
            .await;
        assert!(matches!(outcome, SseLineOutcome::Continue));
        match rx.recv().await.unwrap() {
            StreamEvent::TextDelta(text) => assert_eq!(text, "hello"),
            other => panic!("unexpected event: {other:?}"),
        }

        let outcome = handler.process_line("data: [DONE]", &tx).await;
        assert!(matches!(outcome, SseLineOutcome::Done));
        match rx.recv().await.unwrap() {
            StreamEvent::Done(response) => {
                assert_eq!(response.content.as_deref(), Some("hello"));
                assert!(response.tool_calls.is_empty());
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn handler_ignores_non_data_and_malformed_json_then_finishes_on_eof() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(2);
        let mut handler = OpenAiSseHandler::new();

        assert!(matches!(
            handler.process_line(": keepalive", &tx).await,
            SseLineOutcome::Continue
        ));
        assert!(matches!(
            handler.process_line("data: not-json", &tx).await,
            SseLineOutcome::Continue
        ));
        assert!(rx.try_recv().is_err());

        handler.on_eof(&tx).await;
        match rx.recv().await.unwrap() {
            StreamEvent::Done(response) => assert!(response.content.is_none()),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn handler_captures_usage_chunk_into_response() {
        // With stream_options.include_usage, OpenAI-compatible providers emit
        // a final chunk with empty choices and a populated usage object.
        let (tx, mut rx) = tokio::sync::mpsc::channel(4);
        let mut handler = OpenAiSseHandler::new();

        handler
            .process_line(r#"data: {"choices":[{"delta":{"content":"hi"}}]}"#, &tx)
            .await;
        let _ = rx.recv().await; // TextDelta

        let outcome = handler
            .process_line(
                r#"data: {"choices":[],"usage":{"prompt_tokens":1234,"completion_tokens":56,"total_tokens":1290}}"#,
                &tx,
            )
            .await;
        assert!(matches!(outcome, SseLineOutcome::Continue));

        handler.on_eof(&tx).await;
        match rx.recv().await.unwrap() {
            StreamEvent::Done(response) => {
                let usage = response.usage.expect("usage should be captured");
                assert_eq!(usage.prompt_tokens, 1234);
                assert_eq!(usage.completion_tokens, 56);
                assert_eq!(response.content.as_deref(), Some("hi"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
