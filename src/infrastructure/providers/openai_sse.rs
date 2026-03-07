//! Incremental SSE byte-stream parser for OpenAI chat completions.
//!
//! Extracted from `openai.rs` to keep both files under the 750-line limit.
//! Uses the shared SSE pump from [`sse_common`].

use crate::domain::message::{LlmResponse, ToolCall};
use crate::domain::provider::StreamEvent;
use crate::infrastructure::providers::sse_common::{SseHandler, SseLineOutcome, pump_sse};

use super::OpenAiProvider;

/// SSE line handler for OpenAI chat completions.
struct OpenAiSseHandler {
    content: String,
    tool_calls: Vec<ToolCall>,
}

impl OpenAiSseHandler {
    fn new() -> Self {
        Self {
            content: String::new(),
            tool_calls: Vec::new(),
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
            usage: None,
            stop_reason: None,
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
            if let Some(choices) = chunk["choices"].as_array() {
                for choice in choices {
                    let delta = &choice["delta"];
                    if let Some(text) = delta["content"].as_str() {
                        self.content.push_str(text);
                        let _ = tx.send(StreamEvent::TextDelta(text.to_string())).await;
                    }
                    let mut discard = String::new();
                    OpenAiProvider::apply_delta(delta, &mut discard, &mut self.tool_calls);
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
