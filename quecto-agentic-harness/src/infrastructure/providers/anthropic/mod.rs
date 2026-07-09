// Anthropic adapter: impl LlmProvider for AnthropicProvider.
//
// See gap analysis #437 for Anthropic API parity work.

mod claude_code;
mod normalize;
mod usage;

use std::future::Future;
use std::pin::Pin;

use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, Role, StopReason, ToolCall};
use crate::domain::provider::{ChatRequest, LlmProvider, StreamEvent};
use claude_code::{CLAUDE_CODE_VERSION, sanitize_surrogates, to_claude_code_name};

mod anthropic_sse;
mod anthropic_user_msg;

/// Anthropic LLM provider.
#[derive(Debug, Clone)]
pub struct AnthropicProvider {
    api_key: String,
    api_base: String,
    client: reqwest::Client,
    /// Whether the token is an OAuth access token (Bearer auth + OAuth beta headers).
    is_oauth: bool,
    /// Router-facing provider name. Defaults to `"anthropic"`; registry-built
    /// providers set a custom prefix (e.g. `"anthropic-oauth"`) so the same
    /// vendor can appear under multiple distinct routing keys.
    router_name: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String, api_base: Option<String>) -> Self {
        Self::with_client(api_key, api_base, reqwest::Client::new())
    }

    /// Create with a shared `reqwest::Client` (avoids duplicate connection pools).
    pub fn with_client(api_key: String, api_base: Option<String>, client: reqwest::Client) -> Self {
        Self::with_client_and_name(api_key, api_base, client, "anthropic")
    }

    /// Create with a shared client and a custom router-facing name.
    pub fn with_client_and_name(
        api_key: String,
        api_base: Option<String>,
        client: reqwest::Client,
        router_name: impl Into<String>,
    ) -> Self {
        let is_oauth = crate::infrastructure::auth::oauth::is_anthropic_oauth_token(&api_key);
        Self {
            api_key,
            api_base: api_base.unwrap_or_else(|| "https://api.anthropic.com".to_string()),
            client,
            is_oauth,
            router_name: router_name.into(),
        }
    }

    pub fn is_oauth(&self) -> bool {
        self.is_oauth
    }

    // -----------------------------------------------------------------------
    // Headers (#437-1,2,3,7,9,10,11,12)
    // -----------------------------------------------------------------------

    /// Build the `anthropic-beta` header value.
    ///
    /// Always sends `fine-grained-tool-streaming-2025-05-14` and
    /// `interleaved-thinking-2025-05-14` (except for 4.6 models where
    /// interleaved thinking is built-in and the beta is redundant).
    fn build_beta_header(model: &str, is_oauth: bool) -> String {
        let omits_interleaved_beta = Self::model_omits_interleaved_thinking_beta(model);
        let mut betas: Vec<&str> = Vec::new();

        if is_oauth {
            betas.push("claude-code-20250219");
            betas.push("oauth-2025-04-20");
        }

        // Still required despite "GA" status.
        betas.push("fine-grained-tool-streaming-2025-05-14");

        // Omit only for models where interleaved thinking is built in (#437-9).
        if !omits_interleaved_beta {
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

    /// Returns `true` for models that use adaptive thinking.
    ///
    /// These models require adaptive thinking and reject deprecated sampling /
    /// budget-thinking parameters.
    fn model_uses_adaptive_thinking(model: &str) -> bool {
        use crate::domain::message::starts_with_ci;
        starts_with_ci(model, "claude-opus-4-6")
            || starts_with_ci(model, "claude-opus-4-7")
            || starts_with_ci(model, "claude-opus-4-8")
            || starts_with_ci(model, "claude-sonnet-4-6")
            || starts_with_ci(model, "claude-sonnet-5")
            || starts_with_ci(model, "claude-fable-5")
    }

    fn model_omits_interleaved_thinking_beta(model: &str) -> bool {
        use crate::domain::message::starts_with_ci;
        starts_with_ci(model, "claude-opus-4-6")
            || starts_with_ci(model, "claude-sonnet-4-6")
            || starts_with_ci(model, "claude-sonnet-5")
            || starts_with_ci(model, "claude-fable-5")
    }

    // -----------------------------------------------------------------------
    // Request body building
    // -----------------------------------------------------------------------

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
            body["output_config"] = serde_json::json!({"effort": anthropic_effort_str(effort)});
        }
    }

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

    fn build_tool_result_block(m: &Message) -> serde_json::Value {
        let content_value = if m.image_blocks.is_empty() {
            serde_json::Value::String(sanitize_surrogates(&m.content).into_owned())
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

    fn build_assistant_message(m: &Message, is_oauth: bool) -> serde_json::Value {
        claude_code::build_assistant_message(m, is_oauth)
    }

    #[cfg(test)]
    fn build_tool_result_message(m: &Message) -> serde_json::Value {
        serde_json::json!({
            "role": "user",
            "content": [Self::build_tool_result_block(m)],
        })
    }

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

    fn attach_cost(response: &mut LlmResponse, model: &str) {
        if let Some(ref mut usage) = response.usage {
            if let Some(pricing) = crate::domain::message::model_pricing(model) {
                usage.cost = Some(pricing.cost_for(usage));
            }
        }
    }

    fn parse_response(
        body: &serde_json::Value,
        is_oauth: bool,
        tools: &[crate::domain::tool::ToolDefinition],
    ) -> Result<LlmResponse, DomainError> {
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
                    let raw_name = block["name"].as_str().unwrap_or_default().to_string();
                    // Reverse-map canonical tool names for OAuth (#437-4)
                    let name = if is_oauth {
                        claude_code::from_claude_code_name(&raw_name, tools)
                    } else {
                        raw_name
                    };
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

        let usage = body["usage"].as_object().map(usage::parse_usage);

        let stop_reason = body["stop_reason"].as_str().map(StopReason::parse);

        Ok(LlmResponse {
            content,
            tool_calls,
            usage,
            stop_reason,
            thinking_blocks: vec![],
        })
    }
}

// ---------------------------------------------------------------------------
// LlmProvider trait impl
// ---------------------------------------------------------------------------

impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        &self.router_name
    }

    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        let model = request.model.to_string();
        let is_oauth = self.is_oauth;
        let tools_snapshot: Vec<crate::domain::tool::ToolDefinition> = if is_oauth {
            request.tools.to_vec()
        } else {
            vec![]
        };
        let (_system, body) = Self::build_request_body(&request, is_oauth);
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
            let retry_after = crate::infrastructure::providers::sse_common::retry_after_suffix(
                response.headers(),
            );
            let response_text = response
                .text()
                .await
                .map_err(|e| DomainError::Provider(format!("failed to read response: {}", e)))?;

            if status != 200 {
                return Err(DomainError::Provider(format!(
                    "HTTP {} from Anthropic: {}{}",
                    status, response_text, retry_after
                )));
            }

            let response_json: serde_json::Value =
                serde_json::from_str(&response_text).map_err(|e| {
                    DomainError::Provider(format!("failed to parse response JSON: {}", e))
                })?;

            let mut resp = Self::parse_response(&response_json, is_oauth, &tools_snapshot)?;
            Self::attach_cost(&mut resp, &model);
            Ok(resp)
        })
    }

    fn chat_stream(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        let model = request.model.to_string();
        let is_oauth = self.is_oauth;
        let tools_snapshot: Option<Vec<crate::domain::tool::ToolDefinition>> = if is_oauth {
            Some(request.tools.to_vec())
        } else {
            None
        };
        let (_system, mut body) = Self::build_request_body(&request, is_oauth);
        body["stream"] = serde_json::Value::Bool(true);
        let url = format!("{}/v1/messages", self.api_base);
        let cancel = request.cancel_flag.clone();

        Box::pin(async move {
            if cancel.as_ref().is_some_and(|f| f.is_cancelled()) {
                return Err(DomainError::Provider("request cancelled".into()));
            }
            let mut resp = self
                .stream_chat_with_body(anthropic_sse::StreamParams {
                    body,
                    url: &url,
                    model: &model,
                    tool_defs: tools_snapshot,
                })
                .await?;
            Self::attach_cost(&mut resp, &model);
            Ok(resp)
        })
    }

    fn chat_stream_incremental(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + '_>> {
        let model = request.model.to_string();
        let is_oauth = self.is_oauth;
        let tools_snapshot: Option<Vec<crate::domain::tool::ToolDefinition>> = if is_oauth {
            Some(request.tools.to_vec())
        } else {
            None
        };
        let (_system, mut body) = Self::build_request_body(&request, is_oauth);
        body["stream"] = serde_json::Value::Bool(true);
        let url = format!("{}/v1/messages", self.api_base);
        let cancel = request.cancel_flag.clone();

        // Derived Clone carries every field (crucially `router_name`) into the
        // spawned streaming task. The previous code reconstructed the struct
        // inline with a hardcoded `"anthropic"` router_name, resetting
        // registry-built providers (e.g. `anthropic-oauth`) to the default key.
        let provider = self.clone();

        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(64);
            if cancel.as_ref().is_some_and(|f| f.is_cancelled()) {
                let _ = tx
                    .send(StreamEvent::Error("request cancelled".into()))
                    .await;
                return rx;
            }
            tokio::spawn(async move {
                provider
                    .stream_chat_incremental_with_body(anthropic_sse::IncrementalStreamParams {
                        base: anthropic_sse::StreamParams {
                            body,
                            url: &url,
                            model: &model,
                            tool_defs: tools_snapshot,
                        },
                        tx,
                    })
                    .await;
            });

            rx
        })
    }
}

/// Map an effort level onto Anthropic's documented vocabulary
/// (`low`/`medium`/`high`/`max`). The OpenAI-only levels (#1066) clamp to
/// the nearest documented Anthropic value; Anthropic's own levels are
/// transmitted verbatim, unchanged from the pre-#1066 behaviour.
fn anthropic_effort_str(effort: crate::domain::provider::EffortLevel) -> &'static str {
    use crate::domain::provider::EffortLevel;
    match effort {
        EffortLevel::None => "low",
        EffortLevel::XHigh => "high",
        other => other.as_str(),
    }
}

#[cfg(any(test, feature = "test-support"))]
mod test_support;

#[cfg(test)]
#[path = "anthropic_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "anthropic_parity_tests.rs"]
mod parity_tests;

#[cfg(test)]
#[path = "anthropic_effort_1066_tests.rs"]
mod effort_1066_tests;
