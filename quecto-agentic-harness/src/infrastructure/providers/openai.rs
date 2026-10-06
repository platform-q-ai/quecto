use crate::infrastructure::providers::attempt_profile::{Profile, Surface, Vendor};
// OpenAI adapter: impl LlmProvider for OpenAiProvider.

use super::stream_idle::StreamIdle;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use super::openai_images;
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::domain::error::DomainError;
use crate::domain::inference::value_objects::provider::StreamEvent;
use crate::domain::message::{LlmResponse, Message, Role, ToolCall};
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
    attempt_admission: Option<std::sync::Arc<dyn crate::application::ports::AttemptAdmission>>,
    /// Account ID for OAuth tokens (chatgpt_account_id from JWT).
    account_id: Option<String>,
    /// The bound on a silent streaming reply (#2210).
    stream_idle: StreamIdle,
}

impl OpenAiProvider {
    /// Bound its replies by `stream_idle`: a provider's configured stream
    /// idle limit (#2433 review), or the defaults.
    pub fn with_stream_idle(mut self, stream_idle: StreamIdle) -> Self {
        self.stream_idle = stream_idle;
        self
    }

    /// Bind leaf attempts to an authenticated gate and explicit single-send client.
    ///
    /// The supplied safe client replaces the disabled transport intentionally;
    /// construct it from your configured builder to retain proxy/TLS/timeouts.
    pub fn with_attempt_admission(
        mut self,
        gate: std::sync::Arc<dyn crate::application::ports::AttemptAdmission>,
        client: crate::infrastructure::providers::SingleAttemptClient,
    ) -> Self {
        self.client = client.into_client();
        self.attempt_admission = Some(gate);
        self
    }

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
            attempt_admission: None,
            account_id,
            stream_idle: StreamIdle::default(),
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
    fn build_request_body(&self, request: &ChatRequest<'_>) -> serde_json::Value {
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
        // #2349 review M1: chat completions rejects a call without its result
        // and a result without its call (400); leave both out, as the
        // Responses route does (`codex_input`).
        let (paired, orphans) =
            crate::domain::sessions::entities::session::filter_orphan_tool_pairs(messages);
        if orphans.has_orphans() {
            tracing::warn!(
                orphaned_calls = ?orphans.orphaned_calls,
                orphaned_results = ?orphans.orphaned_results,
                "chat completions: orphaned tool calls/results left out of the request"
            );
        }
        let is_paired = |id: &str| paired.contains(id);
        let sent: Vec<(&Message, serde_json::Value)> = messages
            .iter()
            .filter(|m| match (&m.role, m.tool_call_id.as_deref()) {
                (Role::Tool, Some(id)) => is_paired(id),
                // An assistant turn left with no text (whitespace is none)
                // and no paired call is an empty turn: left out, as
                // `codex_input` does (#2434). Its reasoning is never replayed.
                (Role::Assistant, _) => {
                    m.tool_calls.iter().any(|tc| is_paired(&tc.id)) || !m.content.trim().is_empty()
                }
                _ => true,
            })
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
                // #2421: a user message's images are content parts.
                if let (Role::User, Some(parts)) = (&m.role, openai_images::user_content(m)) {
                    obj["content"] = parts;
                }
                if m.tool_calls.iter().any(|tc| is_paired(&tc.id)) {
                    let tcs: Vec<serde_json::Value> = m
                        .tool_calls
                        .iter()
                        .filter(|tc| is_paired(&tc.id))
                        .map(|tc| {
                            serde_json::json!({
                                "id": tc.id,
                                "type": "function",
                                "function": {
                                    "name": tc.name,
                                    "arguments": tc.wire_arguments(),
                                }
                            })
                        })
                        .collect();
                    obj["tool_calls"] = serde_json::Value::Array(tcs);
                }
                if let Some(ref id) = m.tool_call_id {
                    obj["tool_call_id"] = serde_json::Value::String(id.clone());
                }
                (m, obj)
            })
            .collect();
        // #2421: a tool message carries text only; a batch's images follow it.
        let msgs = openai_images::with_tool_images(sent);

        let mut body = serde_json::json!({
            "model": model,
            "messages": msgs,
            "max_completion_tokens": max_tokens,
        });

        // #1996: a selected effort is transmitted verbatim as the
        // chat-completions `reasoning_effort` parameter (Fireworks, xAI and
        // compatible endpoints document the same name). The application
        // admits a level only from the model's own catalogue vocabulary, so
        // this adapter never decides support and never sends a reasoning
        // option to a model that declared none. Fireworks' alternative
        // `thinking` parameter is mutually exclusive with this one and is
        // never emitted here. OpenAI's own Chat Completions endpoint is the
        // one exception: it rejects `reasoning_effort` with function tools
        // (its reasoning ids are routed to the Responses API, and the OAuth
        // fallback without an account id lands here), so the `openai`
        // adapter never transmits one.
        if let Some(effort) = request.effort.filter(|_| self.transmits_reasoning_effort()) {
            body["reasoning_effort"] = serde_json::Value::String(effort.as_str().to_string());
        }

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

    /// Whether this endpoint accepts `reasoning_effort` on chat completions:
    /// every OpenAI-compatible provider except OpenAI's own (`openai`).
    fn transmits_reasoning_effort(&self) -> bool {
        self.provider_name != "openai"
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
            .map(|thinking| {
                let mut capped = String::new();
                append_visible_thinking(&mut capped, thinking, "OpenAI non-stream reasoning")?;
                Ok(vec![crate::domain::message::ThinkingBlock::Normal {
                    thinking: capped,
                    signature: String::new(),
                }])
            })
            .transpose()?
            .unwrap_or_default();

        let mut tool_calls = Vec::new();
        if let Some(tcs) = message["tool_calls"].as_array() {
            for tc in tcs {
                let id = tc["id"].as_str().unwrap_or_default().to_string();
                let name = tc["function"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                // Most providers send the arguments as a JSON string; some
                // OpenAI-compatible ones send the object itself (#2123).
                let arguments = match &tc["function"]["arguments"] {
                    serde_json::Value::String(text) => text.clone(),
                    serde_json::Value::Object(_) => tc["function"]["arguments"].to_string(),
                    _ => String::new(),
                };
                tool_calls.push(ToolCall {
                    id,
                    name,
                    arguments,
                });
            }
        }

        let usage = body["usage"]
            .as_object()
            .map(crate::infrastructure::providers::usage::parse_openai_usage);

        Ok(LlmResponse {
            content,
            tool_calls,
            usage,
            stop_reason: openai_sse_parser::choice_stop_reason(choice),
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
        model: &str,
        trace: Option<
            std::sync::Arc<crate::domain::inference::events::request_observation::RequestTrace>,
        >,
    ) -> Result<LlmResponse, DomainError> {
        // Observed beside the request, never altering it (#2151).
        let handler = openai_sse::OpenAiSseHandler::with_model(model).with_trace(trace.clone());
        let attempt = super::attempt_transport::PassiveAttempt::begin(
            trace,
            Profile::new(Vendor::OpenAi, Surface::Assembled, self.stream_idle),
        );
        let request_builder = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&body);
        let request_builder = self.apply_auth_headers(request_builder);

        let response = match self.stream_idle.within(request_builder.send()).await {
            Ok(sent) => sent
                .inspect_err(|_| attempt.iter().for_each(|a| a.send_failed()))
                .map_err(|e| {
                    DomainError::Provider(format!(
                        "HTTP error: {}",
                        super::sse_common::format_send_error(&e)
                    ))
                })?,
            Err(silent) => {
                attempt.iter().for_each(|a| a.idle());
                return Err(DomainError::Provider(silent.to_string()));
            }
        };

        if let Some(attempt) = &attempt {
            attempt.response(&response);
        }
        let status = response.status().as_u16();
        if status != 200 {
            let retry_after = super::sse_common::retry_after_suffix(response.headers());
            let read = self.stream_idle.text(response).await;
            attempt.iter().for_each(|a| a.error_read(status, &read));
            let text = super::stream_idle::error_text(read);
            return Err(DomainError::Provider(format!(
                "HTTP {} from OpenAI: {}{}",
                status, text, retry_after
            )));
        }

        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        let pump = AbortOnDrop::new(tokio::spawn(openai_sse::pump_sse_response_for_model(
            response,
            tx,
            handler,
            attempt,
            self.stream_idle,
        )));
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
        model: &str,
        trace: Option<
            std::sync::Arc<crate::domain::inference::events::request_observation::RequestTrace>,
        >,
    ) {
        // Observed beside the request, never altering it (#2151).
        let handler = openai_sse::OpenAiSseHandler::with_model(model).with_trace(trace.clone());
        let attempt = super::attempt_transport::PassiveAttempt::begin(
            trace,
            Profile::new(Vendor::OpenAi, Surface::Incremental, self.stream_idle),
        );
        let request_builder = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .json(&body);
        let request_builder = self.apply_auth_headers(request_builder);
        let mut response = match self.stream_idle.within(request_builder.send()).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                attempt.iter().for_each(|a| a.send_failed());
                let _ = tx
                    .send(StreamEvent::Error(format!(
                        "HTTP error: {}",
                        super::sse_common::format_send_error(&e)
                    )))
                    .await;
                return;
            }
            Err(silent) => {
                attempt.iter().for_each(|a| a.idle());
                let _ = tx.send(StreamEvent::Error(silent.to_string())).await;
                return;
            }
        };
        if let Some(attempt) = &attempt {
            attempt.response(&response);
        }
        let status = response.status().as_u16();
        if status != 200 {
            let retry_after = super::sse_common::retry_after_suffix(response.headers());
            let read = self.stream_idle.text(response).await;
            attempt.iter().for_each(|a| a.error_read(status, &read));
            let text = super::sse_common::truncate_error_body(super::stream_idle::error_text(read));
            let _ = tx
                .send(StreamEvent::Error(format!(
                    "HTTP {status} from OpenAI: {text}{retry_after}"
                )))
                .await;
            return;
        }
        let idle = self.stream_idle;
        openai_sse::pump_sse_bytes_for_model(&mut response, &tx, handler, attempt, idle).await;
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
    fn route_order(&self) -> Vec<String> {
        vec![self.name().to_string()]
    }
    fn name(&self) -> &str {
        &self.provider_name
    }

    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        let cancel = request.cancel_flag.clone();
        let trace = request.trace.clone();
        let model = request.model.to_string();
        let body = self.build_request_body(&request);
        let url = format!("{}/chat/completions", self.api_base);

        Box::pin(async move {
            let request_builder = self
                .client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(&body);
            let request_builder = self.apply_auth_headers(request_builder);

            if let Some(gate) = &self.attempt_admission {
                return super::attempt_transport::text(
                    gate,
                    trace.clone(),
                    cancel.as_ref(),
                    request_builder,
                    Profile::new(Vendor::OpenAi, Surface::Chat, self.stream_idle),
                    |text| {
                        let json = serde_json::from_str(text).map_err(|e| {
                            DomainError::Provider(format!("failed to parse response JSON: {e}"))
                        })?;
                        let mut parsed = Self::parse_response(&json)?;
                        crate::domain::inference::services::usage_accounting::attach_cost(
                            &mut parsed,
                            &model,
                        );
                        Ok(parsed)
                    },
                )
                .await;
            }

            // Observed beside the request, never altering it (#2151). A
            // whole reply at once has no first token to time.
            let attempt = super::attempt_transport::PassiveAttempt::begin(
                trace,
                Profile::new(Vendor::OpenAi, Surface::Chat, self.stream_idle),
            );
            // A whole reply sends nothing until complete: bounded in total.
            let exchange = async {
                let response = request_builder
                    .send()
                    .await
                    .inspect_err(|_| attempt.iter().for_each(|a| a.send_failed()))
                    .map_err(|e| {
                        DomainError::Provider(format!(
                            "HTTP error: {}",
                            super::sse_common::format_send_error(&e)
                        ))
                    })?;
                if let Some(attempt) = &attempt {
                    attempt.response(&response);
                }
                let status = response.status().as_u16();
                let retry_after = super::sse_common::retry_after_suffix(response.headers());
                let text = response
                    .text()
                    .await
                    .inspect_err(|_| attempt.iter().for_each(|a| a.read_failed()))
                    .map_err(|e| {
                        DomainError::Provider(format!("failed to read response: {}", e))
                    })?;
                Ok::<_, DomainError>((status, retry_after, text))
            };
            let (status, retry_after, response_text) = match self.stream_idle.whole(exchange).await
            {
                Ok(read) => read?,
                Err(late) => {
                    attempt.iter().for_each(|a| a.timed_out());
                    return Err(DomainError::Provider(late.to_string()));
                }
            };

            if status != 200 {
                attempt
                    .iter()
                    .for_each(|a| a.http_error(status, Some(&response_text)));
                return Err(DomainError::Provider(format!(
                    "HTTP {} from OpenAI: {}{}",
                    status, response_text, retry_after
                )));
            }

            let rejected = || attempt.iter().for_each(|a| a.rejected());
            let response_json: serde_json::Value = serde_json::from_str(&response_text)
                .inspect_err(|_| rejected())
                .map_err(|e| {
                    DomainError::Provider(format!("failed to parse response JSON: {}", e))
                })?;

            let mut parsed = Self::parse_response(&response_json).inspect_err(|_| rejected())?;
            crate::domain::inference::services::usage_accounting::attach_cost(&mut parsed, &model);
            attempt.iter().for_each(|a| a.completed());
            Ok(parsed)
        })
    }

    fn chat_stream(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        let cancel = request.cancel_flag.clone();
        let trace = request.trace.clone();
        let model = request.model.to_string();
        let mut body = self.build_request_body(&request);
        body["stream"] = serde_json::Value::Bool(true);
        // Ask OpenAI-compatible providers (OpenAI, Fireworks, …) to emit a
        // final usage chunk so we report exact context tokens instead of a
        // heuristic estimate.
        body["stream_options"] = serde_json::json!({ "include_usage": true });
        let url = format!("{}/chat/completions", self.api_base);
        Box::pin(async move {
            if let Some(gate) = &self.attempt_admission {
                let builder = self.apply_auth_headers(self.client.post(&url).json(&body));
                let (tx, rx) = tokio::sync::mpsc::channel(64);
                let pump = super::attempt_transport::stream(
                    gate,
                    (trace.clone(), cancel.as_ref()),
                    builder,
                    Profile::new(Vendor::OpenAi, Surface::Assembled, self.stream_idle),
                    tx,
                    openai_sse::OpenAiSseHandler::with_model(&model).with_trace(trace.clone()),
                );
                let (_, result) = tokio::join!(pump, super::attempt_transport::collect(rx));
                return result;
            }
            self.stream_chat_with_body(body, &url, &model, trace).await
        })
    }

    fn chat_stream_incremental(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + '_>> {
        let cancel = request.cancel_flag.clone();
        let trace = request.trace.clone();
        let model = request.model.to_string();
        let mut body = self.build_request_body(&request);
        body["stream"] = serde_json::Value::Bool(true);
        // Request a final usage chunk (see `chat_stream`).
        body["stream_options"] = serde_json::json!({ "include_usage": true });
        let url = format!("{}/chat/completions", self.api_base);
        let provider = self.clone();
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(64);
            super::attempt_transport::queue_before_spawn(
                provider.attempt_admission.as_ref(),
                trace.as_ref(),
            );
            tokio::spawn(async move {
                if let Some(gate) = &provider.attempt_admission {
                    let builder =
                        provider.apply_auth_headers(provider.client.post(&url).json(&body));
                    super::attempt_transport::stream(
                        gate,
                        (trace.clone(), cancel.as_ref()),
                        builder,
                        Profile::new(Vendor::OpenAi, Surface::Incremental, provider.stream_idle),
                        tx,
                        openai_sse::OpenAiSseHandler::with_model(&model).with_trace(trace.clone()),
                    )
                    .await;
                } else {
                    provider
                        .pump_sse_incremental(body, &url, tx, &model, trace)
                        .await;
                }
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

// #2434: the request after a turn that ended with no reply.
#[cfg(test)]
#[path = "openai_2434_tests.rs"]
mod empty_reply_2434_tests;
// #2414 review L7: consecutive user messages on the wire.
#[cfg(test)]
#[path = "openai_consecutive_user_tests.rs"]
mod consecutive_user_tests;
#[cfg(test)]
#[path = "openai_effort_1996_tests.rs"]
mod effort_1996_tests;
#[cfg(test)]
#[path = "openai_2421_tests.rs"]
mod images_2421_tests;
#[cfg(test)]
#[path = "openai_tests.rs"]
mod tests;

#[cfg(any(test, feature = "test-support"))]
impl OpenAiProvider {
    /// Bound a silent streaming reply by `limit` rather than the default
    /// stream idle limit (tests, #2210).
    pub fn with_stream_idle_limit(mut self, limit: std::time::Duration) -> Self {
        let total = self.stream_idle.total();
        self.stream_idle =
            crate::infrastructure::providers::stream_idle::StreamIdle::new(limit).with_total(total);
        self
    }

    /// Bound a whole non-streaming reply by `total` rather than the default
    /// reply total limit (tests, #2210 review).
    pub fn with_reply_total_limit(mut self, total: std::time::Duration) -> Self {
        self.stream_idle = self.stream_idle.with_total(total);
        self
    }

    /// Public accessor for the chat-completions request builder (BDD and
    /// integration tests, #1996).
    pub fn build_chat_completions_body_for_test(
        provider_name: &str,
        request: &ChatRequest<'_>,
    ) -> serde_json::Value {
        Self::with_client_and_name(
            provider_name,
            "sk-test".to_string(),
            None,
            reqwest::Client::new(),
        )
        .build_request_body(request)
    }
}
