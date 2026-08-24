// Anthropic SSE accumulator and parser — extracted from anthropic.rs to keep
// individual files under the 750-line quality gate.
//
// #437: Added `signature_delta` handling for thinking block signature capture.

use crate::domain::message::{LlmResponse, StopReason, ThinkingBlock, ToolCall, UsageInfo};
use crate::domain::provider::StreamEvent;
use crate::domain::tool::ToolDefinition;
use crate::domain::visible_thinking::{MAX_VISIBLE_THINKING_BYTES, append_visible_thinking};
use crate::infrastructure::providers::sse_limits::append_with_limit;

/// Accumulates Anthropic SSE events into a final [`LlmResponse`].
#[derive(Default)]
pub(super) struct SseAccumulator {
    /// When `Some`, reverse-maps PascalCase canonical tool names (e.g. `"Read"`)
    /// back to registry names (e.g. `"read"`) using case-insensitive matching.
    /// Set for OAuth mode; `None` for API key mode (no remapping).
    tool_defs: Option<Vec<ToolDefinition>>,
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
    // Thinking block accumulation (#437-5,6)
    thinking_blocks: Vec<ThinkingBlock>,
    current_thinking: String,
    current_thinking_signature: String,
    in_thinking: bool,
    in_redacted_thinking: bool,
    current_redacted_data: String,
}

impl SseAccumulator {
    /// Create an accumulator that reverse-maps tool names via `tool_defs`.
    pub(super) fn with_tool_defs(tool_defs: Vec<ToolDefinition>) -> Self {
        Self {
            tool_defs: Some(tool_defs),
            ..Default::default()
        }
    }

    /// Reverse-map a wire tool name to the registry name.
    /// Returns the original name unchanged when no tool_defs are configured.
    pub(super) fn remap_tool_name(&self, raw: &str) -> String {
        match &self.tool_defs {
            Some(defs) => {
                let remapped = super::claude_code::from_claude_code_name(raw, defs);
                if remapped == raw {
                    tracing::debug!(
                        raw_name = raw,
                        "SSE tool name not in tool_defs, passing through"
                    );
                }
                remapped
            }
            None => raw.to_string(),
        }
    }

    pub(super) fn handle_message_start(&mut self, chunk: &serde_json::Value) {
        if let Some(usage) = chunk["message"]["usage"].as_object() {
            self.update_usage_fields(usage);
        }
    }

    pub(super) fn handle_block_start(&mut self, chunk: &serde_json::Value) {
        let block = &chunk["content_block"];
        match block["type"].as_str() {
            Some("tool_use") => {
                self.current_tool_id = block["id"].as_str().unwrap_or_default().to_string();
                let raw_name = block["name"].as_str().unwrap_or_default();
                self.current_tool_name = self.remap_tool_name(raw_name);
                self.current_tool_input.clear();
                self.in_tool_input = true;
            }
            Some("thinking") => {
                self.current_thinking.clear();
                self.current_thinking_signature.clear();
                self.in_thinking = true;
                self.in_redacted_thinking = false;
            }
            Some("redacted_thinking") => {
                // Redacted thinking blocks carry the opaque `data` payload
                // in the content_block_start event itself.
                self.in_redacted_thinking = true;
                self.in_thinking = false;
                self.current_redacted_data = block["data"].as_str().unwrap_or_default().to_string();
            }
            _ => {}
        }
    }

    pub(super) fn handle_block_delta(
        &mut self,
        chunk: &serde_json::Value,
    ) -> Result<(), DomainError> {
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
            Some("thinking_delta") => {
                if let Some(thinking) = delta["thinking"].as_str() {
                    append_visible_thinking(
                        &mut self.current_thinking,
                        thinking,
                        "Anthropic SSE thinking",
                    )?;
                }
            }
            Some("signature_delta") => {
                // #437-6: Capture thinking block signature for multi-turn replay.
                if let Some(sig) = delta["signature"].as_str() {
                    append_with_limit(
                        &mut self.current_thinking_signature,
                        sig,
                        MAX_VISIBLE_THINKING_BYTES,
                        "Anthropic SSE thinking signature",
                    )?;
                }
            }
            _ => {}
        }
        Ok(())
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
        if self.in_thinking {
            let thinking = std::mem::take(&mut self.current_thinking);
            let signature = std::mem::take(&mut self.current_thinking_signature);
            if !thinking.trim().is_empty() || !signature.is_empty() {
                self.thinking_blocks.push(ThinkingBlock::Normal {
                    thinking,
                    signature,
                });
            }
            self.in_thinking = false;
        }
        if self.in_redacted_thinking {
            let data = std::mem::take(&mut self.current_redacted_data);
            if !data.is_empty() {
                self.thinking_blocks.push(ThinkingBlock::Redacted { data });
            }
            self.in_redacted_thinking = false;
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

    pub(super) fn has_observable_output(&self) -> bool {
        !self.content.is_empty()
            || !self.tool_calls.is_empty()
            || !self.thinking_blocks.is_empty()
            || self.in_tool_input
            || self.in_thinking
            || self.in_redacted_thinking
    }

    /// Consume `self` and return the assembled [`LlmResponse`].
    pub(super) fn into_response(self) -> LlmResponse {
        let content = if self.content.is_empty() {
            None
        } else {
            Some(self.content)
        };
        let usage = if self.prompt_tokens.is_some() || self.completion_tokens.is_some() {
            let prompt_tokens = self.prompt_tokens.unwrap_or(0);
            Some(UsageInfo {
                prompt_tokens,
                completion_tokens: self.completion_tokens.unwrap_or(0),
                cache_read_tokens: self.cache_read_tokens,
                cache_write_tokens: self.cache_write_tokens,
                // Anthropic reports `input_tokens` as the non-cached delta only;
                // true context occupancy adds the cached read + creation tokens.
                context_tokens: Some(super::usage::context_input_tokens(
                    prompt_tokens,
                    self.cache_read_tokens,
                    self.cache_write_tokens,
                )),
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
            thinking_blocks: self.thinking_blocks,
        }
    }

    /// Extract accumulated thinking blocks for test assertions.
    ///
    /// Used by `test_sse_signature_delta_accumulates_signature` to verify
    /// that `signature_delta` events are correctly accumulated.
    #[cfg(test)]
    pub(super) fn thinking_blocks(&self) -> &[ThinkingBlock] {
        &self.thinking_blocks
    }

    pub(super) fn try_stream_event_from_block_delta(
        &mut self,
        chunk: &serde_json::Value,
    ) -> Option<StreamEvent> {
        let delta = &chunk["delta"];
        match delta["type"].as_str() {
            Some("text_delta") => delta["text"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| StreamEvent::TextDelta(s.to_string())),
            Some("thinking_delta") => {
                let thinking = delta["thinking"].as_str().filter(|s| !s.is_empty())?;
                // Budget-check only. Persistence happens in `handle_block_delta`
                // so live + persist share one append.
                let remaining = crate::domain::visible_thinking::MAX_VISIBLE_THINKING_BYTES
                    .saturating_sub(self.current_thinking.len());
                if thinking.len() > remaining {
                    return None;
                }
                Some(StreamEvent::ThinkingDelta(thinking.to_string()))
            }
            Some("input_json_delta") => delta["partial_json"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| StreamEvent::ToolCallDelta(s.to_string())),
            _ => None,
        }
    }
}

/// Emit a [`StreamEvent`] for a single `content_block_delta` SSE event.
#[cfg(test)]
pub(super) fn stream_event_from_delta(delta: &serde_json::Value) -> Option<StreamEvent> {
    match delta["type"].as_str() {
        Some("text_delta") => {
            let text = delta["text"].as_str().filter(|s| !s.is_empty())?;
            Some(StreamEvent::TextDelta(text.to_string()))
        }
        Some("thinking_delta") => {
            let thinking = delta["thinking"].as_str().filter(|s| !s.is_empty())?;
            let mut capped = String::new();
            if append_visible_thinking(&mut capped, thinking, "Anthropic SSE thinking").is_err() {
                return None;
            }
            Some(StreamEvent::ThinkingDelta(thinking.to_string()))
        }
        Some("input_json_delta") => {
            let partial = delta["partial_json"].as_str().filter(|s| !s.is_empty())?;
            Some(StreamEvent::ToolCallDelta(partial.to_string()))
        }
        // signature_delta is handled by the accumulator, no StreamEvent emitted.
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Streaming impl block for AnthropicProvider
// ---------------------------------------------------------------------------

use super::AnthropicProvider;
use crate::domain::error::DomainError;

/// Parameters for streaming requests (avoids 5+ arg methods).
pub(super) struct StreamParams<'a> {
    pub body: serde_json::Value,
    pub url: &'a str,
    pub model: &'a str,
    /// Tool definitions for reverse-mapping OAuth tool names (#438).
    pub tool_defs: Option<Vec<ToolDefinition>>,
}

/// Parameters for incremental streaming (extends [`StreamParams`] with a channel).
pub(super) struct IncrementalStreamParams<'a> {
    pub base: StreamParams<'a>,
    pub tx: tokio::sync::mpsc::Sender<StreamEvent>,
}

impl AnthropicProvider {
    /// Send a streaming chat request with a pre-built JSON body.
    pub(super) async fn stream_chat_with_body(
        &self,
        params: StreamParams<'_>,
    ) -> Result<LlmResponse, DomainError> {
        let request_builder = self.client.post(params.url).json(&params.body);
        let request_builder = self.apply_headers(request_builder, params.model);

        let response = request_builder
            .send()
            .await
            .map_err(|e| DomainError::Provider(format!("HTTP error: {}", e)))?;

        let status = response.status().as_u16();
        if status != 200 {
            let retry_after = crate::infrastructure::providers::sse_common::retry_after_suffix(
                response.headers(),
            );
            let text = response.text().await.unwrap_or_default();
            return Err(DomainError::Provider(format!(
                "HTTP {} from Anthropic: {}{}",
                status, text, retry_after
            )));
        }

        let full = response
            .text()
            .await
            .map_err(|e| DomainError::Provider(format!("failed to read stream: {}", e)))?;

        Self::parse_sse_response(&full, params.tool_defs)
    }

    /// Parse Anthropic SSE events into an assembled [`LlmResponse`].
    ///
    /// When `tool_defs` is `Some`, PascalCase tool names from the API are
    /// reverse-mapped to registry names (OAuth mode, #438).
    pub(super) fn parse_sse_response(
        raw: &str,
        tool_defs: Option<Vec<ToolDefinition>>,
    ) -> Result<LlmResponse, DomainError> {
        let mut acc = match tool_defs {
            Some(defs) => SseAccumulator::with_tool_defs(defs),
            None => SseAccumulator::default(),
        };
        let mut current_event = String::new();
        let mut saw_terminal = false;

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
                "content_block_delta" => acc.handle_block_delta(&chunk)?,
                "content_block_stop" => acc.handle_block_stop(),
                "message_delta" => acc.handle_message_delta(&chunk),
                "message_stop" => {
                    saw_terminal = true;
                    break;
                }
                "error" => {
                    return Err(DomainError::Provider(format_anthropic_stream_error(&chunk)));
                }
                _ => {}
            }
        }

        if !saw_terminal && !acc.has_observable_output() {
            return Err(DomainError::Provider(
                "Anthropic stream ended without completion".to_string(),
            ));
        }

        Ok(acc.into_response())
    }

    /// Perform an HTTP streaming request and pipe SSE bytes into a channel.
    ///
    /// `model` is passed for header construction.
    pub(super) async fn stream_chat_incremental_with_body(
        &self,
        params: IncrementalStreamParams<'_>,
    ) {
        let request_builder = self.client.post(params.base.url).json(&params.base.body);
        let request_builder = self.apply_headers(request_builder, params.base.model);
        let tx = params.tx;

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
            let retry_after = crate::infrastructure::providers::sse_common::retry_after_suffix(
                response.headers(),
            );
            let text = crate::infrastructure::providers::sse_common::truncate_error_body(
                response.text().await.unwrap_or_default(),
            );
            let _ = tx
                .send(StreamEvent::Error(format!(
                    "HTTP {} from Anthropic: {}{}",
                    status, text, retry_after
                )))
                .await;
            return;
        }

        let mut handler = AnthropicSseHandler::with_model(params.base.tool_defs, params.base.model);
        crate::infrastructure::providers::sse_common::pump_sse(&mut response, &tx, &mut handler)
            .await;
    }
}

use crate::infrastructure::providers::sse_common::{SseHandler, SseLineOutcome};

/// SSE line handler for the Anthropic Messages API.
pub(crate) struct AnthropicSseHandler {
    current_event: String,
    acc: SseAccumulator,
    saw_terminal: bool,
    model: Option<String>,
}

impl AnthropicSseHandler {
    fn new(tool_defs: Option<Vec<ToolDefinition>>) -> Self {
        Self {
            current_event: String::new(),
            acc: match tool_defs {
                Some(defs) => SseAccumulator::with_tool_defs(defs),
                None => SseAccumulator::default(),
            },
            saw_terminal: false,
            model: None,
        }
    }

    fn with_model(tool_defs: Option<Vec<ToolDefinition>>, model: &str) -> Self {
        let mut handler = Self::new(tool_defs);
        handler.model = Some(model.to_string());
        handler
    }

    fn take_response(&mut self) -> LlmResponse {
        let mut response = std::mem::take(&mut self.acc).into_response();
        if let Some(model) = &self.model {
            crate::domain::usage_accounting::attach_cost(&mut response, model);
        }
        response
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn new_for_test(tool_defs: Option<Vec<ToolDefinition>>) -> Self {
        Self::new(tool_defs)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn into_response(self) -> LlmResponse {
        self.acc.into_response()
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
            if dispatch_sse_event(
                &self.current_event,
                &chunk_val,
                &mut self.acc,
                self.model.as_deref(),
                tx,
            )
            .await
            {
                self.saw_terminal = true;
                return SseLineOutcome::Done;
            }
        }
        SseLineOutcome::Continue
    }

    async fn on_eof(&mut self, tx: &tokio::sync::mpsc::Sender<StreamEvent>) {
        if !self.saw_terminal && !self.acc.has_observable_output() {
            let _ = tx
                .send(StreamEvent::Error(
                    "Anthropic stream ended without completion".to_string(),
                ))
                .await;
            return;
        }
        let response = self.take_response();
        let _ = tx.send(StreamEvent::Done(response)).await;
    }
}

fn format_anthropic_stream_error(chunk: &serde_json::Value) -> String {
    let error = &chunk["error"];
    let kind = error["type"].as_str().unwrap_or("error");
    let message = error["message"]
        .as_str()
        .unwrap_or("Anthropic stream error");
    format!("Anthropic stream error: type={kind}: {message}")
}

/// Dispatch one parsed SSE event to the accumulator and channel.
///
/// Returns `true` when a terminal event is received.
async fn dispatch_sse_event(
    event_type: &str,
    chunk: &serde_json::Value,
    acc: &mut SseAccumulator,
    model: Option<&str>,
    tx: &tokio::sync::mpsc::Sender<StreamEvent>,
) -> bool {
    match event_type {
        "message_start" => acc.handle_message_start(chunk),
        "content_block_start" => {
            let is_redacted_thinking =
                chunk["content_block"]["type"].as_str() == Some("redacted_thinking");
            acc.handle_block_start(chunk);
            if is_redacted_thinking {
                let _ = tx
                    .send(StreamEvent::ThinkingDelta(
                        "[redacted thinking]".to_string(),
                    ))
                    .await;
            }
            emit_tool_call_start(acc, tx).await;
        }
        "content_block_delta" => {
            if let Some(ev) = acc.try_stream_event_from_block_delta(chunk) {
                let _ = tx.send(ev).await;
            }
            if let Err(err) = acc.handle_block_delta(chunk) {
                let _ = tx.send(StreamEvent::Error(err.to_string())).await;
                return true;
            }
        }
        "content_block_stop" => {
            emit_tool_call_end(acc, tx).await;
            acc.handle_block_stop();
        }
        "message_delta" => acc.handle_message_delta(chunk),
        "message_stop" => {
            let mut response = std::mem::take(acc).into_response();
            if let Some(model) = model {
                crate::domain::usage_accounting::attach_cost(&mut response, model);
            }
            let _ = tx.send(StreamEvent::Done(response)).await;
            return true;
        }
        "error" => {
            let _ = tx
                .send(StreamEvent::Error(format_anthropic_stream_error(chunk)))
                .await;
            return true;
        }
        _ => {}
    }
    false
}

/// Emit a [`StreamEvent::ToolCallStart`] if the accumulator just started a tool call.
///
/// Called *after* `handle_block_start` so the remapped name is already stored
/// in `acc.current_tool_name` — avoids a redundant second remap (#438, #440).
async fn emit_tool_call_start(acc: &SseAccumulator, tx: &tokio::sync::mpsc::Sender<StreamEvent>) {
    if acc.in_tool_input {
        let _ = tx
            .send(StreamEvent::ToolCallStart {
                id: acc.current_tool_id.clone(),
                name: acc.current_tool_name.clone(),
            })
            .await;
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

#[cfg(test)]
#[path = "anthropic_sse_cov_tests.rs"]
mod cov_tests;
