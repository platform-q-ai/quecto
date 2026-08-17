// OpenAI adapter: impl LlmProvider for OpenAiProvider.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Role, ToolCall, UsageInfo};
use crate::domain::provider::{ChatRequest, LlmProvider, StreamEvent};
use crate::domain::visible_thinking::append_visible_thinking;

struct AbortOnDrop<T> {
    handle: Option<tokio::task::JoinHandle<T>>,
}

impl<T> AbortOnDrop<T> {
    fn new(handle: tokio::task::JoinHandle<T>) -> Self {
        Self {
            handle: Some(handle),
        }
    }
}

impl<T> Future for AbortOnDrop<T> {
    type Output = Result<T, tokio::task::JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let handle = self
            .handle
            .as_mut()
            .expect("AbortOnDrop polled after completion");
        Pin::new(handle).poll(cx)
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        if let Some(handle) = &self.handle {
            if !handle.is_finished() {
                handle.abort();
            }
        }
    }
}

/// OpenAI-compatible LLM provider.
#[derive(Debug, Clone)]
pub struct OpenAiProvider {
    provider_name: String,
    api_key: String,
    api_base: String,
    client: reqwest::Client,
    /// Account ID for OAuth tokens (chatgpt_account_id from JWT).
    account_id: Option<String>,
}

impl OpenAiProvider {
    pub fn new(api_key: String, api_base: Option<String>) -> Self {
        Self::with_client(api_key, api_base, reqwest::Client::new())
    }

    /// Create with a shared `reqwest::Client` (avoids duplicate connection pools).
    pub fn with_client(api_key: String, api_base: Option<String>, client: reqwest::Client) -> Self {
        Self::with_client_and_name("openai", api_key, api_base, client)
    }

    /// Create an OpenAI-compatible provider with a custom router prefix.
    pub fn with_client_and_name(
        provider_name: &str,
        api_key: String,
        api_base: Option<String>,
        client: reqwest::Client,
    ) -> Self {
        Self::with_client_and_name_and_oauth_headers(provider_name, api_key, api_base, client, true)
    }

    /// Create a provider while explicitly controlling OpenAI OAuth-specific headers.
    pub fn with_client_and_name_and_oauth_headers(
        provider_name: &str,
        api_key: String,
        api_base: Option<String>,
        client: reqwest::Client,
        include_oauth_headers: bool,
    ) -> Self {
        let account_id = include_oauth_headers
            .then(|| crate::infrastructure::auth::oauth::extract_openai_account_id(&api_key))
            .flatten();
        Self {
            provider_name: provider_name.to_string(),
            api_key,
            api_base: api_base.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            client,
            account_id,
        }
    }

    /// Apply auth headers. Adds `chatgpt-account-id` for OAuth JWT tokens.
    fn apply_auth_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let builder = builder.header("Authorization", format!("Bearer {}", self.api_key));
        if let Some(ref account_id) = self.account_id {
            builder.header("chatgpt-account-id", account_id)
        } else {
            builder
        }
    }

    /// Build the JSON request body for OpenAI chat completions.
    fn build_request_body(request: &ChatRequest<'_>) -> serde_json::Value {
        let messages = request.messages;
        let tools = request.tools;
        let model = request.model;
        let max_tokens = request.max_tokens;
        // #938: strict OpenAI-compatible endpoints (Fireworks qwen3p7-plus)
        // reject any non-leading `system` message. Preserve only the *first*
        // `system` message regardless of its position; demote every later one to
        // `user`, prefixing the content with "[system] " to keep the framing.
        // Tracking `seen_system` (rather than `idx > 0`) avoids an implicit
        // "system is always first" invariant: a non-system message preceding the
        // system prompt no longer causes the real system message to be demoted.
        let mut seen_system = false;
        let msgs: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                let is_system = matches!(m.role, Role::System);
                let demote_system = is_system && seen_system;
                if is_system {
                    seen_system = true;
                }
                let role = if demote_system {
                    "user"
                } else {
                    m.role.as_str()
                };
                let content = if demote_system {
                    format!("[system] {}", m.content)
                } else {
                    m.content.clone()
                };
                let mut obj = serde_json::json!({
                    "role": role,
                    "content": content,
                });
                if !m.tool_calls.is_empty() {
                    let tcs: Vec<serde_json::Value> = m
                        .tool_calls
                        .iter()
                        .map(|tc| {
                            serde_json::json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.arguments,
                                }
                            })
                        })
                        .collect();
                    obj["tool_calls"] = serde_json::Value::Array(tcs);
                }
                if let Some(ref id) = m.tool_call_id {
                    obj["tool_call_id"] = serde_json::Value::String(id.clone());
                }
                // image_blocks: OpenAI tool results only support string content;
                // image blocks from `read` on image files are not forwarded here.
                // Use Anthropic provider for image-aware tool results.
                obj
            })
            .collect();

        let mut body = serde_json::json!({
            "model": model,
            "messages": msgs,
            "max_completion_tokens": max_tokens,
        });

        if !tools.is_empty() {
            let tool_defs: Vec<serde_json::Value> = tools
                .iter()
                .map(|t| {
                    let params: serde_json::Value =
                        serde_json::from_str(&t.parameters_schema).unwrap_or_default();
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": params,
                        }
                    })
                })
                .collect();
            body["tools"] = serde_json::Value::Array(tool_defs);
        }

        body
    }

    /// Parse the OpenAI response JSON into our domain LlmResponse.
    fn parse_response(body: &serde_json::Value) -> Result<LlmResponse, DomainError> {
        let choices = body["choices"]
            .as_array()
            .ok_or_else(|| DomainError::Provider("missing choices in response".to_string()))?;

        let choice = choices
            .first()
            .ok_or_else(|| DomainError::Provider("empty choices array".to_string()))?;

        let message = &choice["message"];
        let content = message["content"].as_str().map(|s| s.to_string());
        let thinking_blocks = message
            .get("reasoning")
            .or_else(|| message.get("reasoning_content"))
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .and_then(|thinking| {
                let mut capped = String::new();
                append_visible_thinking(&mut capped, thinking, "OpenAI non-stream reasoning")
                    .ok()?;
                Some(vec![crate::domain::message::ThinkingBlock::Normal {
                    thinking: capped,
                    signature: String::new(),
                }])
            })
            .unwrap_or_default();

        let mut tool_calls = Vec::new();
        if let Some(tcs) = message["tool_calls"].as_array() {
            for tc in tcs {
                let id = tc["id"].as_str().unwrap_or_default().to_string();
                let name = tc["function"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let arguments = tc["function"]["arguments"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                tool_calls.push(ToolCall {
                    id,
                    name,
                    arguments,
                });
            }
        }

        // Non-streaming chat historically reports `context_tokens: None` and lets
        // the gauge fall back to `prompt_tokens` (which already counts the full
        // prompt); only the streaming/SSE paths surface `total_tokens`. Preserve
        // that per-path behaviour after consolidating onto the shared parser.
        let usage = body["usage"]
            .as_object()
            .map(crate::infrastructure::providers::usage::parse_openai_usage)
            .map(|u| UsageInfo {
                context_tokens: None,
                ..u
            });

        Ok(LlmResponse {
            content,
            tool_calls,
            usage,
            stop_reason: None,
            thinking_blocks,
        })
    }
}

impl OpenAiProvider {
    /// Send a streaming chat request with a pre-built JSON body.
    async fn stream_chat_with_body(
        &self,
        body: serde_json::Value,
        url: &str,
    ) -> Result<LlmResponse, DomainError> {
        let request_builder = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&body);
        let request_builder = self.apply_auth_headers(request_builder);

        let response = request_builder.send().await.map_err(|e| {
            DomainError::Provider(format!(
                "HTTP error: {}",
                super::sse_common::format_send_error(&e)
            ))
        })?;

        let status = response.status().as_u16();
        if status != 200 {
            let retry_after = super::sse_common::retry_after_suffix(response.headers());
            let text = response.text().await.unwrap_or_default();
            return Err(DomainError::Provider(format!(
                "HTTP {} from OpenAI: {}{}",
                status, text, retry_after
            )));
        }

        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let pump = AbortOnDrop::new(tokio::spawn(openai_sse::pump_sse_response(response, tx)));
        while let Some(event) = rx.recv().await {
            match event {
                StreamEvent::Done(response) => {
                    let _ = pump.await;
                    return Ok(response);
                }
                StreamEvent::Error(error) => {
                    let _ = pump.await;
                    return Err(DomainError::Provider(error));
                }
                StreamEvent::TextDelta(_)
                | StreamEvent::ThinkingDelta(_)
                | StreamEvent::ToolCallStart { .. }
                | StreamEvent::ToolCallDelta(_)
                | StreamEvent::ToolCallEnd { .. } => {}
            }
        }
        let _ = pump.await;
        Err(DomainError::Provider(
            "OpenAI SSE stream ended without completion".to_string(),
        ))
    }

    #[cfg(test)]
    fn parse_sse_response(raw: &str) -> Result<LlmResponse, DomainError> {
        openai_sse_parser::parse_sse_response(raw)
    }

    /// Consume SSE body incrementally, emitting `StreamEvent`s per delta.
    async fn pump_sse_incremental(
        &self,
        body: serde_json::Value,
        url: &str,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) {
        let request_builder = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&body);
        let request_builder = self.apply_auth_headers(request_builder);
        let mut response = match request_builder.send().await {
            Ok(r) => r,
            Err(e) => {
                let _ = tx
                    .send(StreamEvent::Error(format!(
                        "HTTP error: {}",
                        super::sse_common::format_send_error(&e)
                    )))
                    .await;
                return;
            }
        };
        let status = response.status().as_u16();
        if status != 200 {
            let retry_after = super::sse_common::retry_after_suffix(response.headers());
            let text =
                super::sse_common::truncate_error_body(response.text().await.unwrap_or_default());
            let _ = tx
                .send(StreamEvent::Error(format!(
                    "HTTP {status} from OpenAI: {text}{retry_after}"
                )))
                .await;
            return;
        }
        openai_sse::pump_sse_bytes(&mut response, &tx).await;
    }

    fn apply_delta(
        delta: &serde_json::Value,
        content: &mut String,
        tool_calls: &mut Vec<ToolCall>,
    ) -> Result<(), DomainError> {
        openai_sse_parser::apply_delta(delta, content, tool_calls)
    }
}

impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        &self.provider_name
    }

    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        let body = Self::build_request_body(&request);
        let url = format!("{}/chat/completions", self.api_base);

        Box::pin(async move {
            let request_builder = self
                .client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body);
            let request_builder = self.apply_auth_headers(request_builder);

            let response = request_builder.send().await.map_err(|e| {
                DomainError::Provider(format!(
                    "HTTP error: {}",
                    super::sse_common::format_send_error(&e)
                ))
            })?;

            let status = response.status().as_u16();
            let retry_after = super::sse_common::retry_after_suffix(response.headers());
            let response_text = response
                .text()
                .await
                .map_err(|e| DomainError::Provider(format!("failed to read response: {}", e)))?;

            if status != 200 {
                return Err(DomainError::Provider(format!(
                    "HTTP {} from OpenAI: {}{}",
                    status, response_text, retry_after
                )));
            }

            let response_json: serde_json::Value =
                serde_json::from_str(&response_text).map_err(|e| {
                    DomainError::Provider(format!("failed to parse response JSON: {}", e))
                })?;

            Self::parse_response(&response_json)
        })
    }

    fn chat_stream(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        let mut body = Self::build_request_body(&request);
        body["stream"] = serde_json::Value::Bool(true);
        // Ask OpenAI-compatible providers (OpenAI, Fireworks, …) to emit a
        // final usage chunk so we report exact context tokens instead of a
        // heuristic estimate.
        body["stream_options"] = serde_json::json!({ "include_usage": true });
        let url = format!("{}/chat/completions", self.api_base);
        Box::pin(async move { self.stream_chat_with_body(body, &url).await })
    }

    fn chat_stream_incremental(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + '_>> {
        let mut body = Self::build_request_body(&request);
        body["stream"] = serde_json::Value::Bool(true);
        // Request a final usage chunk (see `chat_stream`).
        body["stream_options"] = serde_json::json!({ "include_usage": true });
        let url = format!("{}/chat/completions", self.api_base);
        let provider = self.clone();
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(64);
            tokio::spawn(async move {
                provider.pump_sse_incremental(body, &url, tx).await;
            });
            rx
        })
    }
}

#[path = "openai_sse.rs"]
pub(super) mod openai_sse;
#[path = "openai_sse_parser.rs"]
pub(crate) mod openai_sse_parser;

#[cfg(test)]
#[path = "openai_cov_tests.rs"]
mod cov_tests;

#[cfg(test)]
#[path = "openai_tests.rs"]
mod tests;
