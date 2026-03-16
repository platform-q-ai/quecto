// Anthropic adapter: impl LlmProvider for AnthropicProvider.
//
// Parity targets: pi-mono (github.com/badlogic/pi-mono) and
// OpenCode (github.com/anomalyco/opencode). See gap analysis #437.

mod claude_code;
mod normalize;

use std::future::Future;
use std::pin::Pin;

use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, Role, StopReason, ToolCall, UsageInfo};
use crate::domain::provider::{ChatRequest, LlmProvider, StreamEvent};
use claude_code::{CLAUDE_CODE_VERSION, sanitize_surrogates, to_claude_code_name};

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

    /// Whether this provider instance uses OAuth authentication.
    pub fn is_oauth(&self) -> bool {
        self.is_oauth
    }

    // -----------------------------------------------------------------------
    // Headers (#437-1,2,3,7,9,10,11,12)
    // -----------------------------------------------------------------------

    /// Build the `anthropic-beta` header value.
    ///
    /// Both Pi and OpenCode always send `fine-grained-tool-streaming-2025-05-14`
    /// and `interleaved-thinking-2025-05-14` (except for 4.6 models where
    /// interleaved thinking is built-in and the beta is redundant).
    fn build_beta_header(model: &str, is_oauth: bool) -> String {
        let adaptive = Self::model_uses_adaptive_thinking(model);
        let mut betas: Vec<&str> = Vec::new();

        if is_oauth {
            betas.push("claude-code-20250219");
            betas.push("oauth-2025-04-20");
        }

        // Both Pi and OpenCode still send this despite "GA" status.
        betas.push("fine-grained-tool-streaming-2025-05-14");

        // Omit for 4.6 models where interleaved thinking is built-in (#437-9).
        if !adaptive {
            betas.push("interleaved-thinking-2025-05-14");
        }

        betas.join(",")
    }

    /// Apply all HTTP headers to a request builder.
    ///
    /// Combines auth, beta, version, and identity headers in one place.
    fn apply_headers(
        &self,
        builder: reqwest::RequestBuilder,
        model: &str,
    ) -> reqwest::RequestBuilder {
        let beta = Self::build_beta_header(model, self.is_oauth);
        let builder = builder
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .header("Accept", "application/json") // #437-10
            .header("anthropic-beta", beta);

        if self.is_oauth {
            builder
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("user-agent", format!("claude-cli/{}", CLAUDE_CODE_VERSION))
                .header("x-app", "cli")
        } else {
            builder.header("x-api-key", &self.api_key)
        }
    }

    // -----------------------------------------------------------------------
    // Model detection
    // -----------------------------------------------------------------------

    /// Returns `true` for models that use adaptive thinking (Opus 4.6, Sonnet 4.6).
    ///
    /// These models deprecate `thinking: {type: "enabled", budget_tokens: N}` in
    /// favour of `thinking: {type: "adaptive"}` with `output_config.effort`.
    fn model_uses_adaptive_thinking(model: &str) -> bool {
        use crate::domain::message::starts_with_ci;
        starts_with_ci(model, "claude-opus-4-6") || starts_with_ci(model, "claude-sonnet-4-6")
    }

    // -----------------------------------------------------------------------
    // Request body building
    // -----------------------------------------------------------------------

    /// Build the JSON request body for Anthropic Messages API.
    ///
    /// `is_oauth` controls Claude Code identity injection (system prompt prefix
    /// and tool name remapping). Passed from `self.is_oauth` in the `LlmProvider`
    /// impl methods.
    /// Apply thinking/temperature/effort configuration to the request body.
    fn apply_thinking_config(
        body: &mut serde_json::Value,
        request: &ChatRequest<'_>,
        adaptive_model: bool,
    ) {
        if adaptive_model {
            body["max_tokens"] = serde_json::json!(request.max_tokens);
            body["thinking"] = serde_json::json!({"type": "adaptive"});
        } else if let Some(level) = request.thinking_level {
            if level.is_adaptive() {
                body["max_tokens"] = serde_json::json!(request.max_tokens);
                body["thinking"] = serde_json::json!({"type": "adaptive"});
            } else {
                let budget = level
                    .budget_tokens()
                    .expect("non-Adaptive level has a budget");
                body["max_tokens"] = serde_json::json!(request.max_tokens.max(budget));
                body["thinking"] = serde_json::json!({
                    "type": "enabled",
                    "budget_tokens": budget,
                });
            }
        } else {
            body["max_tokens"] = serde_json::json!(request.max_tokens);
            body["temperature"] = serde_json::json!(request.temperature);
        }
        let effective_effort = request
            .effort
            .or_else(|| adaptive_model.then_some(crate::domain::provider::EffortLevel::Low));
        if let Some(effort) = effective_effort {
            body["output_config"] = serde_json::json!({"effort": effort.as_str()});
        }
    }

    /// Apply system prompt to the request body (#437-1).
    fn apply_system_prompt(
        body: &mut serde_json::Value,
        system_prompt: &Option<String>,
        is_oauth: bool,
    ) {
        if is_oauth {
            let mut blocks = vec![serde_json::json!({
                "type": "text",
                "text": "You are Claude Code, Anthropic's official CLI for Claude.",
                "cache_control": { "type": "ephemeral" }
            })];
            if let Some(sys) = system_prompt {
                blocks.push(serde_json::json!({
                    "type": "text",
                    "text": sanitize_surrogates(sys),
                    "cache_control": { "type": "ephemeral" }
                }));
            }
            body["system"] = serde_json::Value::Array(blocks);
        } else if let Some(sys) = system_prompt {
            body["system"] = serde_json::json!([{
                "type": "text",
                "text": sanitize_surrogates(sys),
                "cache_control": { "type": "ephemeral" }
            }]);
        }
    }

    fn build_request_body(
        request: &ChatRequest<'_>,
        is_oauth: bool,
    ) -> (Option<String>, serde_json::Value) {
        let (system_prompt, mut api_messages) =
            Self::build_messages(request.messages, request.model, is_oauth);
        let mut body = serde_json::json!({
            "model": request.model,
            "messages": [],
        });

        let adaptive_model = Self::model_uses_adaptive_thinking(request.model);
        Self::apply_thinking_config(&mut body, request, adaptive_model);
        Self::apply_system_prompt(&mut body, &system_prompt, is_oauth);
        Self::apply_cache_control_to_last_user_message(&mut api_messages);

        body["messages"] = serde_json::Value::Array(api_messages);

        if !request.tools.is_empty() {
            body["tools"] =
                serde_json::Value::Array(Self::build_tool_defs(request.tools, is_oauth));
        }

        // tool_choice
        if let Some(ref tc) = request.tool_choice {
            body["tool_choice"] = match tc {
                crate::domain::provider::ToolChoice::Auto => {
                    serde_json::json!({"type": "auto"})
                }
                crate::domain::provider::ToolChoice::Any => {
                    serde_json::json!({"type": "any"})
                }
                crate::domain::provider::ToolChoice::Specific(name) => {
                    let tool_name = if is_oauth {
                        to_claude_code_name(name).to_string()
                    } else {
                        name.clone()
                    };
                    serde_json::json!({"type": "tool", "name": tool_name})
                }
            };
        }

        // metadata
        if let Some(ref meta) = request.metadata {
            if let Some(ref user_id) = meta.user_id {
                body["metadata"] = serde_json::json!({"user_id": user_id});
            }
        }

        (system_prompt, body)
    }

    /// Apply `cache_control: { type: "ephemeral" }` to the last user message.
    fn apply_cache_control_to_last_user_message(api_messages: &mut [serde_json::Value]) {
        let last_user_idx = api_messages
            .iter()
            .rposition(|m| m["role"].as_str() == Some("user"));

        if let Some(idx) = last_user_idx {
            let msg = &mut api_messages[idx];
            let content = msg
                .get_mut("content")
                .expect("user message must have content");

            if content.is_string() {
                let text = content.as_str().unwrap_or("").to_string();
                *content = serde_json::json!([{
                    "type": "text",
                    "text": text,
                    "cache_control": { "type": "ephemeral" }
                }]);
            } else if let Some(blocks) = content.as_array_mut() {
                if let Some(last_block) = blocks.last_mut() {
                    last_block["cache_control"] = serde_json::json!({"type": "ephemeral"});
                }
            }
        }
    }

    fn normalize_messages(messages: &[Message]) -> Vec<std::borrow::Cow<'_, Message>> {
        normalize::normalize_messages(messages)
    }

    /// Convert domain messages to Anthropic API message format.
    ///
    /// Applies normalization, batches consecutive tool results, and handles
    /// thinking block replay (#437-5) and tool name remapping (#437-4).
    fn build_messages(
        messages: &[Message],
        model: &str,
        is_oauth: bool,
    ) -> (Option<String>, Vec<serde_json::Value>) {
        let supports_vision = anthropic_user_msg::model_supports_vision(model);
        let normalized = Self::normalize_messages(messages);
        let mut system_prompt: Option<String> = None;
        let mut api_messages: Vec<serde_json::Value> = Vec::new();
        let mut i = 0;

        while i < normalized.len() {
            let m: &Message = &normalized[i];
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
                    api_messages.push(Self::build_assistant_message(m, is_oauth));
                    i += 1;
                }
                Role::Tool => {
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

        normalize::inject_orphaned_tool_results(&mut api_messages);
        (system_prompt, api_messages)
    }

    /// Build a single `tool_result` content block.
    fn build_tool_result_block(m: &Message) -> serde_json::Value {
        let content_value = if m.image_blocks.is_empty() {
            serde_json::Value::String(sanitize_surrogates(&m.content))
        } else {
            let mut result_content: Vec<serde_json::Value> =
                vec![serde_json::json!({"type": "text", "text": sanitize_surrogates(&m.content)})];
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

    /// Build an Anthropic assistant message with thinking blocks and tool name
    /// remapping. Delegated to `claude_code::build_assistant_message` (#437-4,5).
    fn build_assistant_message(m: &Message, is_oauth: bool) -> serde_json::Value {
        claude_code::build_assistant_message(m, is_oauth)
    }

    /// Build an Anthropic tool_result as a complete user message (single tool result).
    #[cfg(test)]
    fn build_tool_result_message(m: &Message) -> serde_json::Value {
        serde_json::json!({
            "role": "user",
            "content": [Self::build_tool_result_block(m)],
        })
    }

    /// Build Anthropic tool definitions with optional name remapping (#437-4).
    fn build_tool_defs(
        tools: &[crate::domain::tool::ToolDefinition],
        is_oauth: bool,
    ) -> Vec<serde_json::Value> {
        tools
            .iter()
            .map(|t| {
                let input_schema: serde_json::Value =
                    serde_json::from_str(&t.parameters_schema).unwrap_or_default();
                let name = if is_oauth {
                    to_claude_code_name(&t.name).to_string()
                } else {
                    t.name.to_string()
                };
                serde_json::json!({
                    "name": name,
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

// ---------------------------------------------------------------------------
// LlmProvider trait impl
// ---------------------------------------------------------------------------

impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        let model = request.model.to_string();
        let (_system, body) = Self::build_request_body(&request, self.is_oauth);
        let url = format!("{}/v1/messages", self.api_base);
        let cancel = request.cancel_flag.clone();

        Box::pin(async move {
            if cancel.as_ref().is_some_and(|f| f.is_cancelled()) {
                return Err(DomainError::Provider("request cancelled".into()));
            }
            let request_builder = self.client.post(&url).json(&body);
            let request_builder = self.apply_headers(request_builder, &model);

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
        let (_system, mut body) = Self::build_request_body(&request, self.is_oauth);
        body["stream"] = serde_json::Value::Bool(true);
        let url = format!("{}/v1/messages", self.api_base);
        let cancel = request.cancel_flag.clone();

        Box::pin(async move {
            if cancel.as_ref().is_some_and(|f| f.is_cancelled()) {
                return Err(DomainError::Provider("request cancelled".into()));
            }
            let mut resp = self.stream_chat_with_body(body, &url, &model).await?;
            Self::attach_cost(&mut resp, &model);
            Ok(resp)
        })
    }

    fn chat_stream_incremental(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + '_>> {
        let model = request.model.to_string();
        let (_system, mut body) = Self::build_request_body(&request, self.is_oauth);
        body["stream"] = serde_json::Value::Bool(true);
        let url = format!("{}/v1/messages", self.api_base);
        let cancel = request.cancel_flag.clone();

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
            tokio::spawn(async move {
                let provider = AnthropicProvider {
                    api_key,
                    api_base,
                    client,
                    is_oauth,
                };
                provider
                    .stream_chat_incremental_with_body(anthropic_sse::IncrementalStreamParams {
                        body,
                        url: &url,
                        model: &model,
                        tx,
                    })
                    .await;
            });

            rx
        })
    }
}

// ---------------------------------------------------------------------------
// Public test-support methods
// ---------------------------------------------------------------------------

#[cfg(any(test, feature = "test-support"))]
impl AnthropicProvider {
    pub fn build_request_body_public(
        request: &ChatRequest<'_>,
    ) -> (Option<String>, serde_json::Value) {
        Self::build_request_body(request, false)
    }

    /// Build request body with explicit `is_oauth` flag (for OAuth-specific tests).
    pub fn build_request_body_with_oauth(
        request: &ChatRequest<'_>,
        is_oauth: bool,
    ) -> (Option<String>, serde_json::Value) {
        Self::build_request_body(request, is_oauth)
    }

    pub fn build_messages_public(messages: &[Message]) -> (Option<String>, Vec<serde_json::Value>) {
        Self::build_messages(messages, "claude-opus-4-5", false)
    }

    pub fn build_messages_for_model_public(
        messages: &[Message],
        model: &str,
    ) -> (Option<String>, Vec<serde_json::Value>) {
        Self::build_messages(messages, model, false)
    }

    /// Public wrapper for `parse_sse_response` (for BDD tests).
    pub fn parse_sse_response_public(raw: &str) -> Result<LlmResponse, DomainError> {
        Self::parse_sse_response(raw)
    }

    /// Parse Anthropic SSE text into a sequence of [`StreamEvent`]s.
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

    pub fn parse_sse_events_public(raw: &str) -> Vec<StreamEvent> {
        Self::parse_sse_events(raw)
    }

    pub fn build_tool_result_message_public(m: &Message) -> serde_json::Value {
        serde_json::json!({
            "role": "user",
            "content": [Self::build_tool_result_block(m)],
        })
    }

    /// Public helper: build beta header for testing.
    pub fn build_beta_header_public(model: &str, is_oauth: bool) -> String {
        Self::build_beta_header(model, is_oauth)
    }

    /// Public helper: convert tool name to Claude Code canonical casing.
    pub fn to_claude_code_name_public(name: &str) -> &str {
        to_claude_code_name(name)
    }

    /// Public helper: reverse tool name mapping.
    pub fn from_claude_code_name_public(
        name: &str,
        tool_defs: &[crate::domain::tool::ToolDefinition],
    ) -> String {
        claude_code::from_claude_code_name(name, tool_defs)
    }
}

#[cfg(test)]
#[path = "anthropic_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "anthropic_parity_tests.rs"]
mod parity_tests;
