//! Incremental SSE byte-stream parser for OpenAI chat completions.
//!
//! Extracted from `openai.rs` to keep both files under the 750-line limit.

use crate::domain::message::{LlmResponse, ToolCall};
use crate::domain::provider::StreamEvent;

use super::OpenAiProvider;

/// Accumulator for incremental SSE parsing of OpenAI chat completions.
struct Accum {
    content: String,
    tool_calls: Vec<ToolCall>,
}

enum Outcome {
    Continue,
    Done,
}

impl Accum {
    fn new() -> Self {
        Self {
            content: String::new(),
            tool_calls: Vec::new(),
        }
    }

    /// Process a single SSE `data:` payload, emitting `TextDelta` events.
    async fn process_data(
        &mut self,
        data: &str,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Outcome {
        if data == "[DONE]" {
            return Outcome::Done;
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
        Outcome::Continue
    }

    fn into_response(self) -> LlmResponse {
        let content = if self.content.is_empty() {
            None
        } else {
            Some(self.content)
        };
        LlmResponse {
            content,
            tool_calls: self.tool_calls,
            usage: None,
            stop_reason: None,
        }
    }
}

/// Maximum SSE line buffer before rejecting a misbehaving server.
const MAX_LINE_BYTES: usize = 1024 * 1024; // 1 MiB

/// Append a chunk to the carry buffer, guarding against runaway lines.
async fn extend_carry(
    carry: &mut Vec<u8>,
    bytes: &[u8],
    tx: &tokio::sync::mpsc::Sender<StreamEvent>,
) -> bool {
    if carry.len() + bytes.len() > MAX_LINE_BYTES && !carry.contains(&b'\n') {
        let _ = tx
            .send(StreamEvent::Error("SSE line exceeds 1 MiB limit".into()))
            .await;
        return false;
    }
    carry.extend_from_slice(bytes);
    true
}

/// Drain complete lines from `carry`, returning `true` on `[DONE]`.
async fn drain_lines(
    carry: &mut Vec<u8>,
    accum: &mut Accum,
    tx: &tokio::sync::mpsc::Sender<StreamEvent>,
) -> bool {
    while let Some(pos) = carry.iter().position(|&b| b == b'\n') {
        let raw_line = carry.drain(..=pos).collect::<Vec<u8>>();
        if let Ok(line) = std::str::from_utf8(&raw_line) {
            let line = line.trim();
            if let Some(data) = line.strip_prefix("data: ") {
                if matches!(accum.process_data(data, tx).await, Outcome::Done) {
                    return true;
                }
            }
        }
    }
    false
}

/// Consume an OpenAI SSE byte stream, emitting `StreamEvent`s per delta.
pub(crate) async fn pump_sse_bytes(
    response: &mut reqwest::Response,
    tx: &tokio::sync::mpsc::Sender<StreamEvent>,
) {
    let mut carry: Vec<u8> = Vec::new();
    let mut accum = Accum::new();

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
        if !extend_carry(&mut carry, &bytes, tx).await {
            return;
        }
        if drain_lines(&mut carry, &mut accum, tx).await {
            let _ = tx.send(StreamEvent::Done(accum.into_response())).await;
            return;
        }
    }
    let _ = tx.send(StreamEvent::Done(accum.into_response())).await;
}
