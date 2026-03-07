// Anthropic SSE accumulator and parser — extracted from anthropic.rs to keep
// individual files under the 750-line quality gate.

use crate::domain::message::{LlmResponse, StopReason, ToolCall, UsageInfo};
use crate::domain::provider::StreamEvent;

/// Accumulates Anthropic SSE events into a final [`LlmResponse`].
///
/// Both the buffered (`parse_sse_response`) and the true incremental
/// (`stream_chat_incremental_with_body`) code paths use this type to build
/// the assembled response from event fragments.
#[derive(Default)]
pub(super) struct SseAccumulator {
    content: String,
    tool_calls: Vec<ToolCall>,
    pub(super) current_tool_id: String,
    pub(super) current_tool_name: String,
    pub(super) current_tool_input: String,
    pub(super) in_tool_input: bool,
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
    cache_read_tokens: Option<u32>,
    cache_write_tokens: Option<u32>,
    stop_reason: Option<StopReason>,
}

impl SseAccumulator {
    pub(super) fn handle_message_start(&mut self, chunk: &serde_json::Value) {
        if let Some(usage) = chunk["message"]["usage"].as_object() {
            self.update_usage_fields(usage);
        }
    }

    pub(super) fn handle_block_start(&mut self, chunk: &serde_json::Value) {
        if chunk["content_block"]["type"].as_str() == Some("tool_use") {
            self.current_tool_id = chunk["content_block"]["id"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            self.current_tool_name = chunk["content_block"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            self.current_tool_input.clear();
            self.in_tool_input = true;
        }
    }

    pub(super) fn handle_block_delta(&mut self, chunk: &serde_json::Value) {
        let delta = &chunk["delta"];
        match delta["type"].as_str() {
            Some("text_delta") => {
                if let Some(text) = delta["text"].as_str() {
                    self.content.push_str(text);
                }
            }
            Some("input_json_delta") => {
                if let Some(json) = delta["partial_json"].as_str() {
                    self.current_tool_input.push_str(json);
                }
            }
            _ => {}
        }
    }

    pub(super) fn handle_block_stop(&mut self) {
        if self.in_tool_input {
            self.tool_calls.push(ToolCall {
                id: std::mem::take(&mut self.current_tool_id),
                name: std::mem::take(&mut self.current_tool_name),
                arguments: std::mem::take(&mut self.current_tool_input),
            });
            self.in_tool_input = false;
        }
    }

    pub(super) fn handle_message_delta(&mut self, chunk: &serde_json::Value) {
        if let Some(reason) = chunk["delta"]["stop_reason"].as_str() {
            self.stop_reason = Some(StopReason::parse(reason));
        }
        if let Some(usage) = chunk["usage"].as_object() {
            if let Some(v) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
                self.completion_tokens = Some(v as u32);
            }
            if let Some(v) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
                self.prompt_tokens = Some(v as u32);
            }
        }
    }

    /// Extract usage fields from an Anthropic usage object.
    fn update_usage_fields(&mut self, usage: &serde_json::Map<String, serde_json::Value>) {
        if let Some(v) = usage.get("input_tokens").and_then(|v| v.as_u64()) {
            self.prompt_tokens = Some(v as u32);
        }
        if let Some(v) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
            self.completion_tokens = Some(v as u32);
        }
        if let Some(v) = usage
            .get("cache_read_input_tokens")
            .and_then(|v| v.as_u64())
        {
            self.cache_read_tokens = Some(v as u32);
        }
        if let Some(v) = usage
            .get("cache_creation_input_tokens")
            .and_then(|v| v.as_u64())
        {
            self.cache_write_tokens = Some(v as u32);
        }
    }

    /// Consume `self` and return the assembled [`LlmResponse`].
    pub(super) fn into_response(self) -> LlmResponse {
        let content = if self.content.is_empty() {
            None
        } else {
            Some(self.content)
        };
        let usage = if self.prompt_tokens.is_some() || self.completion_tokens.is_some() {
            Some(UsageInfo {
                prompt_tokens: self.prompt_tokens.unwrap_or(0),
                completion_tokens: self.completion_tokens.unwrap_or(0),
                cache_read_tokens: self.cache_read_tokens,
                cache_write_tokens: self.cache_write_tokens,
                cost: None,
            })
        } else {
            None
        };
        LlmResponse {
            content,
            tool_calls: self.tool_calls,
            usage,
            stop_reason: self.stop_reason,
        }
    }
}

/// Emit a [`StreamEvent`] for a single `content_block_delta` SSE event.
///
/// Returns `Some(event)` when the delta type is known and the payload is
/// non-empty; returns `None` for unknown delta types or empty payloads.
pub(super) fn stream_event_from_delta(delta: &serde_json::Value) -> Option<StreamEvent> {
    match delta["type"].as_str() {
        Some("text_delta") => {
            let text = delta["text"].as_str().filter(|s| !s.is_empty())?;
            Some(StreamEvent::TextDelta(text.to_string()))
        }
        Some("thinking_delta") => {
            let thinking = delta["thinking"].as_str().filter(|s| !s.is_empty())?;
            Some(StreamEvent::ThinkingDelta(thinking.to_string()))
        }
        Some("input_json_delta") => {
            let partial = delta["partial_json"].as_str().filter(|s| !s.is_empty())?;
            Some(StreamEvent::ToolCallDelta(partial.to_string()))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Streaming impl block for AnthropicProvider
// Placed here to keep anthropic/mod.rs within the 750-line quality gate.
// ---------------------------------------------------------------------------

use super::AnthropicProvider;
use crate::domain::error::DomainError;

impl AnthropicProvider {
    /// Send a streaming chat request with a pre-built JSON body.
    ///
    /// Reads the **entire** SSE response as text, then parses events
    /// post-hoc.  Used by `chat_stream()`; for true incremental delivery
    /// use `stream_chat_incremental_with_body()`.
    pub(super) async fn stream_chat_with_body(
        &self,
        body: serde_json::Value,
        url: &str,
    ) -> Result<LlmResponse, DomainError> {
        let request_builder = self
            .client
            .post(url)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body);
        let request_builder = self.apply_auth_headers(request_builder);

        let response = request_builder
            .send()
            .await
            .map_err(|e| DomainError::Provider(format!("HTTP error: {}", e)))?;

        let status = response.status().as_u16();
        if status != 200 {
            let text = response.text().await.unwrap_or_default();
            return Err(DomainError::Provider(format!(
                "HTTP {} from Anthropic: {}",
                status, text
            )));
        }

        let full = response
            .text()
            .await
            .map_err(|e| DomainError::Provider(format!("failed to read stream: {}", e)))?;

        Self::parse_sse_response(&full)
    }

    /// Parse Anthropic SSE events into an assembled [`LlmResponse`].
    pub(super) fn parse_sse_response(raw: &str) -> Result<LlmResponse, DomainError> {
        let mut acc = SseAccumulator::default();
        let mut current_event = String::new();

        for line in raw.lines() {
            let line = line.trim();
            if let Some(event) = line.strip_prefix("event: ") {
                current_event = event.to_string();
                continue;
            }
            if !line.starts_with("data: ") {
                continue;
            }
            let data = &line[6..];
            let chunk: serde_json::Value = serde_json::from_str(data).unwrap_or_default();

            match current_event.as_str() {
                "message_start" => acc.handle_message_start(&chunk),
                "content_block_start" => acc.handle_block_start(&chunk),
                "content_block_delta" => acc.handle_block_delta(&chunk),
                "content_block_stop" => acc.handle_block_stop(),
                "message_delta" => acc.handle_message_delta(&chunk),
                "message_stop" => break,
                _ => {}
            }
        }

        Ok(acc.into_response())
    }

    /// Perform an HTTP streaming request and pipe SSE bytes into a channel
    /// of [`StreamEvent`]s.
    ///
    /// Uses `reqwest::Response::chunk()` so the body is consumed incrementally
    /// — callers receive events as they arrive, not after the full response is
    /// buffered.
    pub(super) async fn stream_chat_incremental_with_body(
        &self,
        body: serde_json::Value,
        url: &str,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) {
        let request_builder = self
            .client
            .post(url)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body);
        let request_builder = self.apply_auth_headers(request_builder);

        let mut response = match request_builder.send().await {
            Ok(r) => r,
            Err(e) => {
                let _ = tx
                    .send(StreamEvent::Error(format!("HTTP error: {}", e)))
                    .await;
                return;
            }
        };

        let status = response.status().as_u16();
        if status != 200 {
            let text = crate::infrastructure::providers::sse_common::truncate_error_body(
                response.text().await.unwrap_or_default(),
            );
            let _ = tx
                .send(StreamEvent::Error(format!(
                    "HTTP {} from Anthropic: {}",
                    status, text
                )))
                .await;
            return;
        }

        // Delegate byte-stream consumption to the shared SSE pump.
        let mut handler = AnthropicSseHandler::new();
        crate::infrastructure::providers::sse_common::pump_sse(&mut response, &tx, &mut handler)
            .await;
    }
}

use crate::infrastructure::providers::sse_common::{SseHandler, SseLineOutcome};

/// SSE line handler for the Anthropic Messages API.
///
/// Unlike OpenAI/Codex, Anthropic's SSE protocol uses `event:` lines to
/// name the event type before the corresponding `data:` line. This handler
/// tracks the current event type across lines.
struct AnthropicSseHandler {
    current_event: String,
    acc: SseAccumulator,
}

impl AnthropicSseHandler {
    fn new() -> Self {
        Self {
            current_event: String::new(),
            acc: SseAccumulator::default(),
        }
    }
}

impl SseHandler for AnthropicSseHandler {
    async fn process_line(
        &mut self,
        line: &str,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> SseLineOutcome {
        if let Some(event_type) = line.strip_prefix("event: ") {
            self.current_event = event_type.to_string();
        } else if let Some(data) = line.strip_prefix("data: ") {
            let chunk_val: serde_json::Value = serde_json::from_str(data).unwrap_or_default();
            if dispatch_sse_event(&self.current_event, &chunk_val, &mut self.acc, tx).await {
                return SseLineOutcome::Done;
            }
        }
        SseLineOutcome::Continue
    }

    async fn on_eof(&mut self, tx: &tokio::sync::mpsc::Sender<StreamEvent>) {
        let _ = tx
            .send(StreamEvent::Done(
                std::mem::take(&mut self.acc).into_response(),
            ))
            .await;
    }
}

/// Dispatch one parsed SSE event to the accumulator and channel.
///
/// Returns `true` when `message_stop` is received.
async fn dispatch_sse_event(
    event_type: &str,
    chunk: &serde_json::Value,
    acc: &mut SseAccumulator,
    tx: &tokio::sync::mpsc::Sender<StreamEvent>,
) -> bool {
    match event_type {
        "message_start" => acc.handle_message_start(chunk),
        "content_block_start" => {
            emit_tool_call_start(chunk, tx).await;
            acc.handle_block_start(chunk);
        }
        "content_block_delta" => {
            if let Some(ev) = stream_event_from_delta(&chunk["delta"]) {
                let _ = tx.send(ev).await;
            }
            acc.handle_block_delta(chunk);
        }
        "content_block_stop" => {
            emit_tool_call_end(acc, tx).await;
            acc.handle_block_stop();
        }
        "message_delta" => acc.handle_message_delta(chunk),
        "message_stop" => {
            let _ = tx
                .send(StreamEvent::Done(std::mem::take(acc).into_response()))
                .await;
            return true;
        }
        _ => {}
    }
    false
}

/// Emit a [`StreamEvent::ToolCallStart`] if the block is a `tool_use` block.
async fn emit_tool_call_start(
    chunk: &serde_json::Value,
    tx: &tokio::sync::mpsc::Sender<StreamEvent>,
) {
    let block = &chunk["content_block"];
    if block["type"].as_str() == Some("tool_use") {
        let id = block["id"].as_str().unwrap_or_default().to_string();
        let name = block["name"].as_str().unwrap_or_default().to_string();
        let _ = tx.send(StreamEvent::ToolCallStart { id, name }).await;
    }
}

/// Emit a [`StreamEvent::ToolCallEnd`] if the accumulator holds an in-progress tool call.
async fn emit_tool_call_end(acc: &SseAccumulator, tx: &tokio::sync::mpsc::Sender<StreamEvent>) {
    if acc.in_tool_input {
        let _ = tx
            .send(StreamEvent::ToolCallEnd {
                id: acc.current_tool_id.clone(),
                name: acc.current_tool_name.clone(),
                arguments: acc.current_tool_input.clone(),
            })
            .await;
    }
}
