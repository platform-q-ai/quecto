// Anthropic adapter: impl LlmProvider for AnthropicProvider.

use std::future::Future;
use std::pin::Pin;

use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, Role, StopReason, ToolCall, UsageInfo};
use crate::domain::provider::{ChatRequest, LlmProvider, StreamEvent};

mod anthropic_sse;
mod anthropic_user_msg;
#[cfg(any(test, feature = "test-support"))]
use anthropic_sse::SseAccumulator;

/// Anthropic LLM provider.
#[derive(Debug)]
pub struct AnthropicProvider {
    api_key: String,
    api_base: String,
    client: reqwest::Client,
    /// Whether the token is an OAuth access token (Bearer auth + Claude Code headers).
    is_oauth: bool,
}

impl AnthropicProvider {
    pub fn new(api_key: String, api_base: Option<String>) -> Self {
        Self::with_client(api_key, api_base, reqwest::Client::new())
    }

    /// Create with a shared `reqwest::Client` (avoids duplicate connection pools).
    pub fn with_client(api_key: String, api_base: Option<String>, client: reqwest::Client) -> Self {
        let is_oauth = crate::infrastructure::auth::oauth::is_anthropic_oauth_token(&api_key);
        Self {
            api_key,
            api_base: api_base.unwrap_or_else(|| "https://api.anthropic.com".to_string()),
            client,
            is_oauth,
        }
    }

    /// Apply the correct auth headers based on whether this is an OAuth or API key token.
    fn apply_auth_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if self.is_oauth {
            builder
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("anthropic-beta", "claude-code-20250219,oauth-2025-04-20")
                .header("user-agent", "quecto/0.12.0 (external, cli)")
                .header("x-app", "cli")
        } else {
            builder
                .header("x-api-key", &self.api_key)
                .header("anthropic-beta", "fine-grained-tool-streaming-2025-05-14")
        }
    }

    /// Build the JSON request body for Anthropic Messages API.
    fn build_request_body(request: &ChatRequest<'_>) -> (Option<String>, serde_json::Value) {
        let (system_prompt, mut api_messages) =
            Self::build_messages(request.messages, request.model);
        let mut body = serde_json::json!({
            "model": request.model,
            "messages": [],
        });

        // When thinking is enabled, temperature must be excluded (Anthropic API requirement)
        // and max_tokens must be at least budget_tokens.
        if let Some(level) = request.thinking_level {
            let budget = level.budget_tokens();
            body["max_tokens"] = serde_json::json!(request.max_tokens.max(budget));
            body["thinking"] = serde_json::json!({
                "type": "enabled",
                "budget_tokens": budget,
            });
        } else {
            body["max_tokens"] = serde_json::json!(request.max_tokens);
            body["temperature"] = serde_json::json!(request.temperature);
        }

        // System prompt as content block array with cache_control for prompt caching (#176).
        if let Some(ref sys) = system_prompt {
            body["system"] = serde_json::json!([{
                "type": "text",
                "text": sys,
                "cache_control": { "type": "ephemeral" }
            }]);
        }

        Self::apply_cache_control_to_last_user_message(&mut api_messages); // #176

        body["messages"] = serde_json::Value::Array(api_messages);

        if !request.tools.is_empty() {
            body["tools"] = serde_json::Value::Array(Self::build_tool_defs(request.tools));
        }

        // tool_choice (#183)
        if let Some(ref tc) = request.tool_choice {
            body["tool_choice"] = match tc {
                crate::domain::provider::ToolChoice::Auto => {
                    serde_json::json!({"type": "auto"})
                }
                crate::domain::provider::ToolChoice::Any => {
                    serde_json::json!({"type": "any"})
                }
                crate::domain::provider::ToolChoice::Specific(name) => {
                    serde_json::json!({"type": "tool", "name": name})
                }
            };
        }

        // metadata (#186)
        if let Some(ref meta) = request.metadata {
            if let Some(ref user_id) = meta.user_id {
                body["metadata"] = serde_json::json!({"user_id": user_id});
            }
        }

        (system_prompt, body)
    }

    /// Apply `cache_control: { type: "ephemeral" }` to the last user message.
    ///
    /// If the last user message content is a plain string, converts it to a
    /// content block array so cache_control can be attached.
    fn apply_cache_control_to_last_user_message(api_messages: &mut [serde_json::Value]) {
        // Find the last user message by scanning backwards.
        let last_user_idx = api_messages
            .iter()
            .rposition(|m| m["role"].as_str() == Some("user"));

        if let Some(idx) = last_user_idx {
            let msg = &mut api_messages[idx];
            let content = msg
                .get_mut("content")
                .expect("user message must have content");

            if content.is_string() {
                // Convert string to content block array.
                let text = content.as_str().unwrap_or("").to_string();
                *content = serde_json::json!([{
                    "type": "text",
                    "text": text,
                    "cache_control": { "type": "ephemeral" }
                }]);
            } else if let Some(blocks) = content.as_array_mut() {
                // Add cache_control to the last block.
                if let Some(last_block) = blocks.last_mut() {
                    last_block["cache_control"] = serde_json::json!({"type": "ephemeral"});
                }
            }
        }
    }

    /// Normalize a tool call ID: strip `[^a-zA-Z0-9_-]` → `'_'`, truncate to 64 (#184).
    fn normalize_tool_call_id(id: &str) -> String {
        id.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .take(64)
            .collect()
    }

    /// Normalize messages: strip invalid tool call IDs, filter error/aborted
    /// assistant turns and their orphaned tool_result counterparts (#184, #182).
    fn normalize_messages(messages: &[Message]) -> Vec<Message> {
        use crate::domain::message::StopReason;
        use std::collections::{HashMap, HashSet};

        // Collect IDs from dropped assistant turns (error/aborted) so we can
        // also drop their orphaned tool_result counterparts.
        let is_incomplete = |m: &&Message| {
            m.role == Role::Assistant
                && matches!(
                    m.stop_reason,
                    Some(StopReason::Error) | Some(StopReason::Aborted)
                )
        };
        let dropped_tool_ids: HashSet<String> = messages
            .iter()
            .filter(is_incomplete)
            .flat_map(|m| m.tool_calls.iter().map(|tc| tc.id.clone()))
            .collect();

        // Build a map from original tool call ID → normalised ID.
        let id_map: HashMap<String, String> = messages
            .iter()
            .flat_map(|m| m.tool_calls.iter())
            .map(|tc| (tc.id.clone(), Self::normalize_tool_call_id(&tc.id)))
            .collect();

        messages
            .iter()
            .filter(|m| {
                // Drop incomplete assistant turns (error or aborted).
                if m.role == Role::Assistant
                    && matches!(
                        m.stop_reason,
                        Some(StopReason::Error) | Some(StopReason::Aborted)
                    )
                {
                    return false;
                }
                // Drop tool results whose tool call was dropped above.
                if m.role == Role::Tool {
                    if let Some(id) = &m.tool_call_id {
                        if dropped_tool_ids.contains(id) {
                            return false;
                        }
                    }
                }
                true
            })
            .map(|m| {
                let mut out = m.clone();
                // Normalise IDs in tool_use blocks (assistant messages).
                for tc in &mut out.tool_calls {
                    if let Some(norm) = id_map.get(&tc.id) {
                        tc.id = norm.clone();
                    }
                }
                // Normalise IDs in tool_result blocks (tool messages).
                if m.role == Role::Tool {
                    if let Some(orig) = &m.tool_call_id {
                        if let Some(norm) = id_map.get(orig) {
                            out.tool_call_id = Some(norm.clone());
                        }
                    }
                }
                out
            })
            .collect()
    }

    fn collect_tool_use_ids(api_messages: &[serde_json::Value]) -> Vec<String> {
        api_messages
            .iter()
            .filter(|m| m["role"] == "assistant")
            .flat_map(|m| m["content"].as_array().into_iter().flatten())
            .filter(|b| b["type"] == "tool_use")
            .filter_map(|b| b["id"].as_str().map(str::to_string))
            .collect()
    }

    fn collect_tool_result_ids(
        api_messages: &[serde_json::Value],
    ) -> std::collections::HashSet<String> {
        api_messages
            .iter()
            .flat_map(|m| m["content"].as_array().into_iter().flatten())
            .filter(|b| b["type"] == "tool_result")
            .filter_map(|b| b["tool_use_id"].as_str().map(str::to_string))
            .collect()
    }

    /// Synthetic `tool_result` for an orphaned tool call (no non-standard fields).
    fn synthetic_tool_result(tool_use_id: String) -> serde_json::Value {
        serde_json::json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": "No result provided",
            "is_error": true,
        })
    }

    /// Detect orphaned tool calls in `api_messages` (tool_use blocks without a
    /// matching tool_result) and inject synthetic error results.
    ///
    /// A tool call is orphaned when an interrupted session has an assistant
    /// message with tool_use blocks but no subsequent tool result messages.
    /// Sending such a payload to Anthropic causes an API error.
    fn inject_orphaned_tool_results(api_messages: &mut Vec<serde_json::Value>) {
        let pending = Self::collect_tool_use_ids(api_messages);
        let satisfied = Self::collect_tool_result_ids(api_messages);

        let mut synthetic_blocks: Vec<serde_json::Value> = pending
            .into_iter()
            .filter(|id| !satisfied.contains(id))
            .map(Self::synthetic_tool_result)
            .collect();

        if synthetic_blocks.is_empty() {
            return;
        }

        // Append into the last user message only if it already contains
        // tool_result blocks (not a plain text user message — mixing them
        // would produce an invalid payload).
        if let Some(last) = api_messages.last_mut() {
            if last["role"] == "user" {
                let has_tool_results = last["content"]
                    .as_array()
                    .map(|arr| arr.iter().any(|b| b["type"] == "tool_result"))
                    .unwrap_or(false);
                if has_tool_results {
                    if let Some(arr) = last["content"].as_array_mut() {
                        arr.append(&mut synthetic_blocks);
                        return;
                    }
                }
            }
        }
        api_messages.push(serde_json::json!({
            "role": "user",
            "content": synthetic_blocks,
        }));
    }

    /// Convert domain messages to Anthropic API message format.
    ///
    /// Applies the #184 normalization pipeline (ID normalization, orphaned tool
    /// call injection, errored message filtering) then batches consecutive tool
    /// result messages into a single user message (#187).
    /// Applies #188 user message content block support (images + capability filtering).
    fn build_messages(
        messages: &[Message],
        model: &str,
    ) -> (Option<String>, Vec<serde_json::Value>) {
        let supports_vision = anthropic_user_msg::model_supports_vision(model);
        let normalized = Self::normalize_messages(messages);
        let mut system_prompt: Option<String> = None;
        let mut api_messages: Vec<serde_json::Value> = Vec::new();
        let mut i = 0;

        while i < normalized.len() {
            let m = &normalized[i];
            match m.role {
                Role::System => {
                    system_prompt = Some(m.content.clone());
                    i += 1;
                }
                Role::User => {
                    if let Some(content) =
                        anthropic_user_msg::build_user_content(m, supports_vision)
                    {
                        api_messages.push(serde_json::json!({"role": "user", "content": content}));
                    }
                    i += 1;
                }
                Role::Assistant => {
                    api_messages.push(Self::build_assistant_message(m));
                    i += 1;
                }
                Role::Tool => {
                    // Batch consecutive tool results into a single user message (#187).
                    let mut tool_results: Vec<serde_json::Value> = Vec::new();
                    while i < normalized.len() && normalized[i].role == Role::Tool {
                        tool_results.push(Self::build_tool_result_block(&normalized[i]));
                        i += 1;
                    }
                    api_messages.push(serde_json::json!({
                        "role": "user",
                        "content": tool_results,
                    }));
                }
            }
        }

        Self::inject_orphaned_tool_results(&mut api_messages);
        (system_prompt, api_messages)
    }

    /// Build a single `tool_result` content block (without the outer role wrapper).
    ///
    /// Used by `build_messages` to batch consecutive tool results.
    fn build_tool_result_block(m: &Message) -> serde_json::Value {
        let content_value = if m.image_blocks.is_empty() {
            serde_json::Value::String(m.content.clone())
        } else {
            let mut result_content: Vec<serde_json::Value> =
                vec![serde_json::json!({"type": "text", "text": m.content})];
            for img in &m.image_blocks {
                result_content.push(serde_json::json!({
                    "type": "image",
                    "source": {
                        "type": "base64",
                        "media_type": img.mime_type,
                        "data": img.data,
                    }
                }));
            }
            serde_json::Value::Array(result_content)
        };
        serde_json::json!({
            "type": "tool_result",
            "tool_use_id": m.tool_call_id.as_deref().unwrap_or(""),
            "content": content_value,
            "is_error": m.is_error,
        })
    }

    /// Build an Anthropic assistant message (with or without tool_use blocks).
    fn build_assistant_message(m: &Message) -> serde_json::Value {
        if m.tool_calls.is_empty() {
            return serde_json::json!({"role": "assistant", "content": m.content});
        }
        let mut content_blocks: Vec<serde_json::Value> = Vec::new();
        if !m.content.is_empty() {
            content_blocks.push(serde_json::json!({"type": "text", "text": m.content}));
        }
        for tc in &m.tool_calls {
            let input: serde_json::Value = serde_json::from_str(&tc.arguments)
                .ok()
                .filter(|v: &serde_json::Value| v.is_object())
                .unwrap_or_else(|| serde_json::json!({}));
            content_blocks.push(serde_json::json!({
                "type": "tool_use",
                "id": tc.id,
                "name": tc.name,
                "input": input,
            }));
        }
        serde_json::json!({"role": "assistant", "content": content_blocks})
    }

    /// Build an Anthropic tool_result as a complete user message (single tool result).
    ///
    /// Delegates to `build_tool_result_block` and wraps in `{ role: "user" }`.
    /// Note: for batched tool results, `build_messages` uses `build_tool_result_block`
    /// directly to group consecutive results into one user message.
    #[cfg(test)]
    fn build_tool_result_message(m: &Message) -> serde_json::Value {
        serde_json::json!({
            "role": "user",
            "content": [Self::build_tool_result_block(m)],
        })
    }

    /// Build Anthropic tool definitions from domain ToolDefinitions.
    fn build_tool_defs(tools: &[crate::domain::tool::ToolDefinition]) -> Vec<serde_json::Value> {
        tools
            .iter()
            .map(|t| {
                let input_schema: serde_json::Value =
                    serde_json::from_str(&t.parameters_schema).unwrap_or_default();
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": input_schema,
                })
            })
            .collect()
    }

    /// Attach cost info to the response's usage data based on model pricing.
    fn attach_cost(response: &mut LlmResponse, model: &str) {
        if let Some(ref mut usage) = response.usage {
            if let Some(pricing) = crate::domain::message::model_pricing(model) {
                usage.cost = Some(pricing.cost_for(usage));
            }
        }
    }

    /// Parse the Anthropic response JSON into our domain LlmResponse.
    fn parse_response(body: &serde_json::Value) -> Result<LlmResponse, DomainError> {
        let content_blocks = body["content"]
            .as_array()
            .ok_or_else(|| DomainError::Provider("missing content in response".to_string()))?;

        let mut text_parts: Vec<String> = Vec::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        for block in content_blocks {
            match block["type"].as_str() {
                Some("text") => {
                    if let Some(t) = block["text"].as_str() {
                        text_parts.push(t.to_string());
                    }
                }
                Some("tool_use") => {
                    let id = block["id"].as_str().unwrap_or_default().to_string();
                    let name = block["name"].as_str().unwrap_or_default().to_string();
                    let input = &block["input"];
                    let arguments = serde_json::to_string(input).unwrap_or_default();
                    tool_calls.push(ToolCall {
                        id,
                        name,
                        arguments,
                    });
                }
                _ => {}
            }
        }

        let content = if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join(""))
        };

        let usage = body["usage"].as_object().map(|u| UsageInfo {
            prompt_tokens: u["input_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
            cache_read_tokens: u
                .get("cache_read_input_tokens")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32),
            cache_write_tokens: u
                .get("cache_creation_input_tokens")
                .and_then(|v| v.as_u64())
                .map(|v| v as u32),
            cost: None,
        });

        let stop_reason = body["stop_reason"].as_str().map(StopReason::parse);

        Ok(LlmResponse {
            content,
            tool_calls,
            usage,
            stop_reason,
        })
    }
}

impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        let model = request.model.to_string();
        let (_system, body) = Self::build_request_body(&request);
        let url = format!("{}/v1/messages", self.api_base);
        let cancel = request.cancel_flag.clone();

        Box::pin(async move {
            if cancel.as_ref().is_some_and(|f| f.is_cancelled()) {
                return Err(DomainError::Provider("request cancelled".into()));
            }
            let request_builder = self
                .client
                .post(&url)
                .header("anthropic-version", "2023-06-01")
                .header("Content-Type", "application/json")
                .json(&body);
            let request_builder = self.apply_auth_headers(request_builder);

            let response = request_builder
                .send()
                .await
                .map_err(|e| DomainError::Provider(format!("HTTP error: {}", e)))?;

            let status = response.status().as_u16();
            let response_text = response
                .text()
                .await
                .map_err(|e| DomainError::Provider(format!("failed to read response: {}", e)))?;

            if status != 200 {
                return Err(DomainError::Provider(format!(
                    "HTTP {} from Anthropic: {}",
                    status, response_text
                )));
            }

            let response_json: serde_json::Value =
                serde_json::from_str(&response_text).map_err(|e| {
                    DomainError::Provider(format!("failed to parse response JSON: {}", e))
                })?;

            let mut resp = Self::parse_response(&response_json)?;
            Self::attach_cost(&mut resp, &model);
            Ok(resp)
        })
    }

    fn chat_stream(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        let model = request.model.to_string();
        let (_system, mut body) = Self::build_request_body(&request);
        body["stream"] = serde_json::Value::Bool(true);
        let url = format!("{}/v1/messages", self.api_base);
        let cancel = request.cancel_flag.clone();

        Box::pin(async move {
            if cancel.as_ref().is_some_and(|f| f.is_cancelled()) {
                return Err(DomainError::Provider("request cancelled".into()));
            }
            let mut resp = self.stream_chat_with_body(body, &url).await?;
            Self::attach_cost(&mut resp, &model);
            Ok(resp)
        })
    }

    fn chat_stream_incremental(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + '_>> {
        let (_system, mut body) = Self::build_request_body(&request);
        body["stream"] = serde_json::Value::Bool(true);
        let url = format!("{}/v1/messages", self.api_base);
        let cancel = request.cancel_flag.clone();

        // Clone all fields needed so the background task can be 'static.
        let api_key = self.api_key.clone();
        let api_base = self.api_base.clone();
        let is_oauth = self.is_oauth;
        let client = self.client.clone();

        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(64);
            if cancel.as_ref().is_some_and(|f| f.is_cancelled()) {
                let _ = tx
                    .send(StreamEvent::Error("request cancelled".into()))
                    .await;
                return rx;
            }
            // Spawn the pump as a detached task so rx is returned immediately.
            tokio::spawn(async move {
                let provider = AnthropicProvider {
                    api_key,
                    api_base,
                    client,
                    is_oauth,
                };
                provider
                    .stream_chat_incremental_with_body(body, &url, tx)
                    .await;
            });

            rx
        })
    }
}

/// Public test-support methods (BDD steps need access to internal builders).
#[cfg(any(test, feature = "test-support"))]
impl AnthropicProvider {
    pub fn build_request_body_public(
        request: &ChatRequest<'_>,
    ) -> (Option<String>, serde_json::Value) {
        Self::build_request_body(request)
    }

    pub fn build_messages_public(messages: &[Message]) -> (Option<String>, Vec<serde_json::Value>) {
        Self::build_messages(messages, "claude-opus-4-5")
    }

    pub fn build_messages_for_model_public(
        messages: &[Message],
        model: &str,
    ) -> (Option<String>, Vec<serde_json::Value>) {
        Self::build_messages(messages, model)
    }

    /// Public wrapper for `parse_sse_response` (for BDD tests).
    pub fn parse_sse_response_public(raw: &str) -> Result<LlmResponse, DomainError> {
        Self::parse_sse_response(raw)
    }

    /// Parse Anthropic SSE text into a sequence of [`StreamEvent`]s.
    ///
    /// Emits granular events (TextDelta, ToolCallStart, etc.) for each SSE
    /// packet, enabling incremental delivery to callers.  Exposed publicly
    /// only under test builds so the incremental protocol can be unit-tested
    /// without an HTTP server.
    fn parse_sse_events(raw: &str) -> Vec<StreamEvent> {
        let mut events: Vec<StreamEvent> = Vec::new();
        let mut acc = SseAccumulator::default();
        let mut current_event = String::new();

        for line in raw.lines() {
            let line = line.trim();
            if let Some(event_type) = line.strip_prefix("event: ") {
                current_event = event_type.to_string();
                continue;
            }
            if let Some(data) = line.strip_prefix("data: ") {
                let chunk: serde_json::Value = serde_json::from_str(data).unwrap_or_default();
                if Self::collect_sse_event(current_event.as_str(), &chunk, &mut acc, &mut events) {
                    break;
                }
            }
        }

        events.push(StreamEvent::Done(acc.into_response()));
        events
    }

    /// Dispatch one SSE event into the accumulator and event list.
    ///
    /// Returns `true` when `message_stop` is received (caller should stop).
    fn collect_sse_event(
        event_type: &str,
        chunk: &serde_json::Value,
        acc: &mut SseAccumulator,
        events: &mut Vec<StreamEvent>,
    ) -> bool {
        use anthropic_sse::stream_event_from_delta;
        match event_type {
            "message_start" => acc.handle_message_start(chunk),
            "content_block_start" => {
                let block = &chunk["content_block"];
                if block["type"].as_str() == Some("tool_use") {
                    let id = block["id"].as_str().unwrap_or_default().to_string();
                    let name = block["name"].as_str().unwrap_or_default().to_string();
                    events.push(StreamEvent::ToolCallStart { id, name });
                }
                acc.handle_block_start(chunk);
            }
            "content_block_delta" => {
                if let Some(ev) = stream_event_from_delta(&chunk["delta"]) {
                    events.push(ev);
                }
                acc.handle_block_delta(chunk);
            }
            "content_block_stop" => {
                if acc.in_tool_input {
                    events.push(StreamEvent::ToolCallEnd {
                        id: acc.current_tool_id.clone(),
                        name: acc.current_tool_name.clone(),
                        arguments: acc.current_tool_input.clone(),
                    });
                }
                acc.handle_block_stop();
            }
            "message_delta" => acc.handle_message_delta(chunk),
            "message_stop" => return true,
            _ => {}
        }
        false
    }

    /// Public wrapper for `parse_sse_events` (for BDD tests — #181).
    pub fn parse_sse_events_public(raw: &str) -> Vec<StreamEvent> {
        Self::parse_sse_events(raw)
    }

    /// Public wrapper for `build_tool_result_message` (for BDD tests).
    pub fn build_tool_result_message_public(m: &Message) -> serde_json::Value {
        serde_json::json!({
            "role": "user",
            "content": [Self::build_tool_result_block(m)],
        })
    }
}

#[cfg(test)]
#[path = "anthropic_tests.rs"]
mod tests;
