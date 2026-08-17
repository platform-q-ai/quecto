//! Incremental SSE byte-stream parser for OpenAI chat completions.
//!
//! Extracted from `openai.rs` to keep both files under the 750-line limit.
//! Uses the shared SSE pump from [`sse_common`].

use crate::domain::message::{LlmResponse, ThinkingBlock, ToolCall, UsageInfo};
use crate::domain::provider::StreamEvent;
use crate::infrastructure::providers::sse_common::{SseHandler, SseLineOutcome, pump_sse};

use super::OpenAiProvider;
use super::openai_sse_parser::{MAX_OPENAI_SSE_CONTENT_BYTES, append_with_limit};
use crate::domain::visible_thinking::append_visible_thinking;

/// SSE line handler for OpenAI chat completions.
pub(crate) struct OpenAiSseHandler {
    content: String,
    tool_calls: Vec<ToolCall>,
    usage: Option<UsageInfo>,
    reasoning: String,
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
            reasoning: String::new(),
            delta_scratch: String::new(),
        }
    }

    fn take_response(&mut self) -> LlmResponse {
        let content = if self.content.is_empty() {
            None
        } else {
            Some(std::mem::take(&mut self.content))
        };
        let thinking_blocks = if self.reasoning.is_empty() {
            Vec::new()
        } else {
            vec![ThinkingBlock::Normal {
                thinking: std::mem::take(&mut self.reasoning),
                signature: String::new(),
            }]
        };
        LlmResponse {
            content,
            tool_calls: std::mem::take(&mut self.tool_calls),
            usage: self.usage.take(),
            stop_reason: None,
            thinking_blocks,
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
                    if let Some(text) = delta
                        .get("reasoning")
                        .or_else(|| delta.get("reasoning_content"))
                        .and_then(|v| v.as_str())
                    {
                        if let Err(err) = append_visible_thinking(
                            &mut self.reasoning,
                            text,
                            "OpenAI SSE reasoning",
                        ) {
                            let _ = tx.send(StreamEvent::Error(err.to_string())).await;
                            return SseLineOutcome::Done;
                        }
                        let _ = tx.send(StreamEvent::ThinkingDelta(text.to_string())).await;
                    }
                    if let Some(text) = delta.get("content").and_then(|v| v.as_str()) {
                        if let Err(err) = append_with_limit(
                            &mut self.content,
                            text,
                            MAX_OPENAI_SSE_CONTENT_BYTES,
                            "assistant content",
                        ) {
                            let _ = tx.send(StreamEvent::Error(err.to_string())).await;
                            return SseLineOutcome::Done;
                        }
                        let _ = tx.send(StreamEvent::TextDelta(text.to_string())).await;
                    }
                    self.delta_scratch.clear();
                    if let Err(err) = OpenAiProvider::apply_delta(
                        delta,
                        &mut self.delta_scratch,
                        &mut self.tool_calls,
                    ) {
                        let _ = tx.send(StreamEvent::Error(err.to_string())).await;
                        return SseLineOutcome::Done;
                    }
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

/// Consume an owned OpenAI SSE byte stream, emitting `StreamEvent`s per delta.
///
/// This is used by non-incremental `chat_stream`, which drains the event
/// receiver while this pump runs in a task. Keeping the pump concurrent with the
/// drain avoids deadlocking when a response has more deltas than the bounded
/// channel capacity.
pub(crate) async fn pump_sse_response(
    mut response: reqwest::Response,
    tx: tokio::sync::mpsc::Sender<StreamEvent>,
) {
    pump_sse_bytes(&mut response, &tx).await;
}

#[cfg(test)]
#[path = "openai_sse_tests.rs"]
mod tests;
