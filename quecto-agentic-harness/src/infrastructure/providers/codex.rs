// OpenAI Responses API adapter: impl LlmProvider using the Responses wire
// protocol under either auth mode (#1066).
//
// - ChatGPT OAuth tokens (from `auth.openai.com`) only work against
//   `chatgpt.com/backend-api/codex/responses` and require Codex-specific
//   headers (`chatgpt-account-id`, `originator`, ...).
// - API keys use the standard `api.openai.com/v1/responses` endpoint with
//   plain `Authorization: Bearer` auth and none of the OAuth-only headers.

use std::future::Future;
use std::pin::Pin;

use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, Role};
use crate::domain::provider::{ChatRequest, LlmProvider, StreamEvent};

#[path = "codex_sse_state.rs"]
mod codex_sse_state;
use codex_sse_state::SseAccumulator;

/// Default Codex backend base URL for ChatGPT OAuth tokens.
const CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api";

/// Default base URL for API-key auth against the standard Responses API.
const OPENAI_API_BASE_URL: &str = "https://api.openai.com/v1";

/// How a [`CodexProvider`] authenticates, which also selects the endpoint
/// flavour (#1066).
#[derive(Debug, Clone)]
enum ResponsesAuth {
    /// ChatGPT OAuth JWT; requests go to `{base}/codex/responses` with the
    /// Codex backend's OAuth-only headers.
    ChatGptOAuth { account_id: String },
    /// Plain OpenAI API key; requests go to `{base}/responses`.
    ApiKey,
}

/// OpenAI Responses API provider (ChatGPT Codex backend or standard API).
#[derive(Debug, Clone)]
pub struct CodexProvider {
    api_key: String,
    api_base: String,
    client: reqwest::Client,
    auth: ResponsesAuth,
}

impl CodexProvider {
    /// Create a new Codex provider.
    ///
    /// `account_id` is extracted from the OAuth JWT's
    /// `https://api.openai.com/auth` claim.
    pub fn new(api_key: String, account_id: String, api_base: Option<String>) -> Self {
        Self::with_client(api_key, account_id, api_base, reqwest::Client::new())
    }

    /// Create with a shared `reqwest::Client` (avoids duplicate connection pools).
    pub fn with_client(
        api_key: String,
        account_id: String,
        api_base: Option<String>,
        client: reqwest::Client,
    ) -> Self {
        Self {
            api_key,
            api_base: api_base.unwrap_or_else(|| CODEX_BASE_URL.to_string()),
            client,
            auth: ResponsesAuth::ChatGptOAuth { account_id },
        }
    }

    /// Create an API-key-authenticated provider against the standard
    /// Responses API (`{base}/responses`) — no OAuth-only headers (#1066).
    pub fn with_api_key(
        api_key: String,
        api_base: Option<String>,
        client: reqwest::Client,
    ) -> Self {
        Self {
            api_key,
            api_base: api_base.unwrap_or_else(|| OPENAI_API_BASE_URL.to_string()),
            client,
            auth: ResponsesAuth::ApiKey,
        }
    }

    /// The Responses endpoint URL for this auth mode.
    fn responses_url(&self) -> String {
        match self.auth {
            ResponsesAuth::ChatGptOAuth { .. } => format!("{}/codex/responses", self.api_base),
            ResponsesAuth::ApiKey => format!("{}/responses", self.api_base),
        }
    }

    /// Build request headers for the active auth mode.
    fn apply_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let builder = builder
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("accept", "text/event-stream");
        match &self.auth {
            ResponsesAuth::ChatGptOAuth { account_id } => builder
                .header("chatgpt-account-id", account_id)
                .header("OpenAI-Beta", "responses=experimental")
                .header("originator", "codex_cli_rs"),
            ResponsesAuth::ApiKey => builder,
        }
    }

    /// Convert our domain messages into Responses API `input` array.
    ///
    /// Calls [`crate::domain::session::filter_orphan_tool_pairs`] to exclude
    /// mismatched function_call/function_call_output pairs (which would cause
    /// HTTP 400). Logs any orphaned pairs with Codex-specific context.
    fn build_input(messages: &[Message]) -> (Option<String>, Vec<serde_json::Value>) {
        let (valid_pairs, diag) = crate::domain::session::filter_orphan_tool_pairs(messages);
        let last_non_tool_assistant_idx = messages
            .iter()
            .enumerate()
            .rev()
            .find(|(_, m)| matches!(m.role, Role::Assistant) && m.tool_calls.is_empty())
            .map(|(i, _)| i);
        if diag.has_orphans() {
            tracing::warn!(
                orphaned_calls = ?diag.orphaned_calls,
                orphaned_outputs = ?diag.orphaned_results,
                "Codex: orphaned function_call/output pairs removed \
                 (session corrupted mid-turn or by context pruning). \
                 OpenAI and Anthropic have the same pairing constraint."
            );
        }
        let mut instructions: Option<String> = None;
        let mut input = Vec::new();

        for (idx, msg) in messages.iter().enumerate() {
            match msg.role {
                Role::System => match &mut instructions {
                    Some(existing) => {
                        existing.push('\n');
                        existing.push_str(&msg.content);
                    }
                    None => instructions = Some(msg.content.clone()),
                },
                Role::User => {
                    input.push(serde_json::json!({ "role": "user", "content": msg.content }));
                }
                Role::Assistant => {
                    let phase = if Some(idx) == last_non_tool_assistant_idx {
                        "final_answer"
                    } else {
                        "commentary"
                    };
                    if !msg.tool_calls.is_empty() {
                        // Emit only the valid (matched) tool calls.
                        let mut emitted = 0usize;
                        for tc in &msg.tool_calls {
                            if valid_pairs.contains(&tc.id) {
                                input.push(serde_json::json!({
                                    "type": "function_call",
                                    "call_id": tc.id,
                                    "name": tc.name,
                                    "arguments": tc.arguments,
                                }));
                                emitted += 1;
                            }
                        }
                        // If every tool call was orphaned and dropped, fall back to
                        // emitting the assistant text content (if any) so narrative
                        // context is not silently lost.
                        if emitted == 0 && !msg.content.is_empty() {
                            input.push(serde_json::json!({
                                "role": "assistant",
                                "phase": phase,
                                "content": msg.content,
                            }));
                        }
                    } else {
                        input.push(serde_json::json!({
                            "role": "assistant",
                            "phase": phase,
                            "content": msg.content,
                        }));
                    }
                }
                Role::Tool => {
                    if let Some(ref call_id) = msg.tool_call_id {
                        if valid_pairs.contains(call_id) {
                            input.push(serde_json::json!({
                                "type": "function_call_output",
                                "call_id": call_id,
                                "output": msg.content,
                            }));
                        }
                    }
                }
            }
        }

        (instructions, input)
    }

    /// Build the Responses API tool definitions.
    fn build_tools(tools: &[crate::domain::tool::ToolDefinition]) -> Vec<serde_json::Value> {
        tools
            .iter()
            .map(|t| {
                let params: serde_json::Value =
                    serde_json::from_str(&t.parameters_schema).unwrap_or_default();
                serde_json::json!({
                    "type": "function",
                    "name": t.name,
                    "description": t.description,
                    "parameters": params,
                    "strict": false,
                })
            })
            .collect()
    }

    /// Validate request constraints that are specific to the ChatGPT Codex backend.
    ///
    /// The slash check is defense-in-depth: `ProviderRouter` strips the
    /// provider prefix before dispatching here, so a well-formed call never
    /// carries a slash. However callers that bypass `ProviderRouter` (e.g.
    /// tests, future code paths) could pass a provider-qualified name, which
    /// the Codex backend would silently reject with an opaque HTTP 400. The
    /// check surfaces this misconfiguration early with a clear message.
    fn validate_request(&self, request: &ChatRequest<'_>) -> Result<(), DomainError> {
        if request.model.contains('/') {
            return Err(DomainError::Provider(
                "codex provider expects a bare model id (e.g. 'gpt-5.3-codex'), not a provider-qualified name".to_string(),
            ));
        }

        // Only the ChatGPT Codex backend mandates instructions; the standard
        // Responses API accepts requests without a system message (#1066).
        if matches!(self.auth, ResponsesAuth::ChatGptOAuth { .. }) {
            let has_instructions = request
                .messages
                .iter()
                .any(|m| matches!(m.role, Role::System) && !m.content.trim().is_empty());
            if !has_instructions {
                return Err(DomainError::Provider(
                    "codex provider requires instructions; include a non-empty system message (e.g. pass --system)".to_string(),
                ));
            }
        }

        Ok(())
    }

    /// Build the full request body.
    ///
    /// `max_output_tokens` is a standard Responses API parameter that the
    /// ChatGPT Codex backend rejects outright with HTTP 400 `{"detail":
    /// "Unsupported parameter: max_output_tokens"}` (#1233 regression), so
    /// it is emitted only on the API-key path.
    fn build_request_body(request: &ChatRequest<'_>, auth: &ResponsesAuth) -> serde_json::Value {
        let (instructions, input) = Self::build_input(request.messages);

        let mut body = serde_json::json!({
            "model": request.model,
            "input": input,
            "store": false,
            "stream": true,
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            "reasoning": {
                "summary": "auto",
            },
            "include": ["reasoning.encrypted_content"],
        });

        if matches!(auth, ResponsesAuth::ApiKey) {
            body["max_output_tokens"] = serde_json::json!(request.max_tokens);
        }

        // #1066: transmit a configured effort, clamped onto OpenAI's
        // documented scale; when none is configured, omit the field so
        // OpenAI's server default applies — the kernel must not invent a
        // fallback. The same rule applies to `text.verbosity`: only derive it
        // from a configured effort, never hardcode a client-side default.
        if let Some(effort) = request.effort {
            body["reasoning"]["effort"] =
                serde_json::Value::String(Self::reasoning_effort_str(effort).to_string());
            body["text"] = serde_json::json!({ "verbosity": Self::verbosity_str(effort) });
        }

        if let Some(inst) = instructions {
            body["instructions"] = serde_json::Value::String(inst);
        }

        if let Some(session_id) = request.session_id {
            body["prompt_cache_key"] =
                serde_json::Value::String(Self::sanitize_cache_key(session_id));
        }

        let tools = Self::build_tools(request.tools);
        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(tools);
        }

        body
    }

    /// Map an effort level onto OpenAI's documented `reasoning.effort` scale
    /// (`none`/`low`/`medium`/`high`/`xhigh`): levels outside that scale
    /// clamp to the nearest documented value (#1066). `max` is
    /// Anthropic-only, so it clamps to `xhigh` here.
    fn reasoning_effort_str(effort: crate::domain::provider::EffortLevel) -> &'static str {
        use crate::domain::provider::EffortLevel;
        match effort {
            EffortLevel::Max => "xhigh",
            other => other.as_str(),
        }
    }

    /// Map an effort level onto the Responses API `text.verbosity` scale,
    /// which only accepts `low`/`medium`/`high`: levels outside that scale
    /// clamp to the nearest documented value.
    fn verbosity_str(effort: crate::domain::provider::EffortLevel) -> &'static str {
        use crate::domain::provider::EffortLevel;
        match effort {
            EffortLevel::None | EffortLevel::Low => "low",
            EffortLevel::Medium => "medium",
            EffortLevel::High | EffortLevel::XHigh | EffortLevel::Max => "high",
        }
    }

    /// Parse a non-streaming Responses API response.
    #[cfg(test)]
    fn parse_response(body: &serde_json::Value) -> Result<LlmResponse, DomainError> {
        let output = body["output"]
            .as_array()
            .ok_or_else(|| DomainError::Provider("missing output in response".into()))?;

        let mut content: Option<String> = None;
        let mut tool_calls = Vec::new();
        let mut reasoning = String::new();

        for item in output {
            match item["type"].as_str() {
                Some("message") => {
                    // Extract text content from message output
                    if let Some(parts) = item["content"].as_array() {
                        for part in parts {
                            if part["type"].as_str() == Some("output_text") {
                                if let Some(text) = part["text"].as_str() {
                                    match &mut content {
                                        Some(c) => c.push_str(text),
                                        None => content = Some(text.to_string()),
                                    }
                                }
                            }
                        }
                    }
                }
                Some("function_call") => {
                    let call_id = item["call_id"].as_str().unwrap_or_default().to_string();
                    let name = item["name"].as_str().unwrap_or_default().to_string();
                    let arguments = item["arguments"].as_str().unwrap_or_default().to_string();
                    tool_calls.push(crate::domain::message::ToolCall {
                        id: call_id,
                        name,
                        arguments,
                    });
                }
                Some("reasoning") => {
                    codex_sse_state::append_reasoning_summary(item, &mut reasoning)
                }
                _ => {}
            }
        }

        let usage = body["usage"]
            .as_object()
            .map(crate::infrastructure::providers::usage::parse_codex_usage);

        let thinking_blocks = if reasoning.is_empty() {
            Vec::new()
        } else {
            vec![crate::domain::message::ThinkingBlock::Normal {
                thinking: reasoning,
                signature: String::new(),
            }]
        };

        Ok(LlmResponse {
            content,
            tool_calls,
            usage,
            stop_reason: None,
            thinking_blocks,
        })
    }

    /// Parse SSE stream from the Responses API and assemble a complete response.
    fn parse_sse_response(raw: &str) -> Result<LlmResponse, DomainError> {
        let mut acc = SseAccumulator::default();
        let mut saw_terminal = false;

        for line in raw.lines() {
            let line = line.trim();
            if !line.starts_with("data: ") {
                continue;
            }
            let data = &line[6..];
            if data == "[DONE]" {
                saw_terminal = true;
                break;
            }
            if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
                if let Some(error) = Self::format_stream_failure(&event) {
                    return Err(DomainError::Provider(error));
                }
                acc.handle_event(&event);
                if event["type"].as_str() == Some("response.completed") {
                    saw_terminal = true;
                    break;
                }
            }
        }

        if !saw_terminal && !acc.has_observable_output() {
            return Err(DomainError::Provider(
                "Responses stream ended without completion".to_string(),
            ));
        }

        Ok(acc.into_response())
    }

    fn format_stream_failure(event: &serde_json::Value) -> Option<String> {
        match event["type"].as_str()? {
            "response.failed" | "response.incomplete" | "error" => {
                let mut parts = vec![format!(
                    "Responses stream {}",
                    event["type"].as_str().unwrap()
                )];
                if let Some(status) = event["response"]["status"].as_str() {
                    parts.push(format!("status={status}"));
                }
                if let Some(reason) = event["response"]["incomplete_details"]["reason"].as_str() {
                    parts.push(format!("reason={reason}"));
                }
                let error = if event["type"].as_str() == Some("error") {
                    &event["error"]
                } else {
                    &event["response"]["error"]
                };
                if let Some(kind) = error["type"].as_str() {
                    parts.push(format!("type={kind}"));
                }
                if let Some(message) = error["message"].as_str() {
                    parts.push(message.to_string());
                }
                Some(parts.join(": "))
            }
            _ => None,
        }
    }

    /// Sanitize a session key for use as `prompt_cache_key`.
    ///
    /// Session keys may contain user-identifying information (e.g. Telegram
    /// chat IDs in the form `"telegram:12345"`). We hash the raw key with
    /// a simple prefix-preserving strategy: keep only the *type* prefix
    /// (chars before the first `:`) and append an 8-hex-char FNV-1a digest
    /// of the full key. This is opaque to the Codex API while still being
    /// stable across requests with the same session.
    ///
    /// Examples:
    /// - `"cli:default"` → `"cli:5e2b9f3a"` (no PII in original, prefix kept)
    /// - `"uds:agent-1"` → `"uds:7b3f1e9a"` (agent ID hidden)
    fn sanitize_cache_key(key: &str) -> String {
        // FNV-1a 32-bit hash — fast, no deps, deterministic.
        let mut hash: u32 = 0x811c_9dc5;
        for byte in key.bytes() {
            hash ^= byte as u32;
            hash = hash.wrapping_mul(0x0100_0193);
        }
        let prefix = key.split(':').next().unwrap_or("session");
        format!("{prefix}:{hash:08x}")
    }

    /// Consume SSE body incrementally, emitting `StreamEvent`s per delta.
    async fn pump_codex_sse(
        &self,
        url: &str,
        body: serde_json::Value,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) {
        let mut response = match self
            .apply_headers(self.client.post(url))
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let _ = tx
                    .send(StreamEvent::Error(format!(
                        "Codex request failed: {}",
                        super::sse_common::format_send_error(&e)
                    )))
                    .await;
                return;
            }
        };
        let status = response.status().as_u16();
        if status != 200 {
            let text =
                super::sse_common::truncate_error_body(response.text().await.unwrap_or_default());
            let _ = tx
                .send(StreamEvent::Error(format!(
                    "HTTP {status} from Codex: {text}"
                )))
                .await;
            return;
        }
        let mut handler = CodexSseHandler::new();
        super::sse_common::pump_sse(&mut response, &tx, &mut handler).await;
    }

    #[cfg(test)]
    pub(crate) async fn pump_sse_response_for_test(
        mut response: reqwest::Response,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) {
        let mut handler = CodexSseHandler::new();
        super::sse_common::pump_sse(&mut response, &tx, &mut handler).await;
    }

    /// Public accessor for `parse_sse_response` (for BDD/integration tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn parse_sse_response_public(raw: &str) -> Result<LlmResponse, DomainError> {
        Self::parse_sse_response(raw)
    }
}

impl LlmProvider for CodexProvider {
    fn name(&self) -> &str {
        "codex"
    }

    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        if let Err(err) = self.validate_request(&request) {
            return Box::pin(async move { Err(err) });
        }

        let body = Self::build_request_body(&request, &self.auth);
        let url = self.responses_url();

        Box::pin(async move {
            let resp = self
                .apply_headers(self.client.post(&url))
                .json(&body)
                .send()
                .await
                .map_err(|e| {
                    DomainError::Provider(format!(
                        "Codex request failed: {}",
                        super::sse_common::format_send_error(&e)
                    ))
                })?;

            let status = resp.status().as_u16();
            if status != 200 {
                let error_body = resp.text().await.unwrap_or_default();
                return Err(DomainError::Provider(format!(
                    "HTTP {} from Codex: {}",
                    status, error_body
                )));
            }

            let raw = resp
                .text()
                .await
                .map_err(|e| DomainError::Provider(format!("failed to read response: {}", e)))?;

            Self::parse_sse_response(&raw)
        })
    }

    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        self.chat(request)
    }

    fn chat_stream_incremental<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + 'a>> {
        if let Err(err) = self.validate_request(&request) {
            return Box::pin(async move {
                let (tx, rx) = tokio::sync::mpsc::channel(1);
                let _ = tx.send(StreamEvent::Error(err.to_string())).await;
                rx
            });
        }
        let body = Self::build_request_body(&request, &self.auth);
        let url = self.responses_url();
        let provider = self.clone();
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(64);
            tokio::spawn(async move {
                provider.pump_codex_sse(&url, body, tx).await;
            });
            rx
        })
    }
}

use super::sse_common::{SseHandler, SseLineOutcome};

/// SSE line handler for the Codex Responses API.
struct CodexSseHandler {
    acc: SseAccumulator,
    saw_terminal: bool,
}

impl CodexSseHandler {
    fn new() -> Self {
        Self {
            acc: SseAccumulator::default(),
            saw_terminal: false,
        }
    }

    fn take_response(&mut self) -> LlmResponse {
        std::mem::take(&mut self.acc).into_response()
    }
}

impl SseHandler for CodexSseHandler {
    async fn process_line(
        &mut self,
        line: &str,
        tx: &tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> SseLineOutcome {
        let Some(data) = line.strip_prefix("data: ") else {
            return SseLineOutcome::Continue;
        };
        if data == "[DONE]" {
            self.saw_terminal = true;
            let _ = tx.send(StreamEvent::Done(self.take_response())).await;
            return SseLineOutcome::Done;
        }
        if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
            if let Some(error) = CodexProvider::format_stream_failure(&event) {
                self.saw_terminal = true;
                let _ = tx.send(StreamEvent::Error(error)).await;
                return SseLineOutcome::Done;
            }
            match event["type"].as_str() {
                Some("response.output_text.delta") => {
                    if let Some(delta) = event["delta"].as_str() {
                        let _ = tx.send(StreamEvent::TextDelta(delta.to_string())).await;
                    }
                }
                Some("response.reasoning_summary_text.delta")
                | Some("response.reasoning.summary_text.delta") => {
                    if let Some(delta) = event["delta"].as_str() {
                        if codex_sse_state::append_reasoning_with_limit(
                            &mut self.acc.reasoning,
                            delta,
                        )
                        .is_ok()
                        {
                            let _ = tx.send(StreamEvent::ThinkingDelta(delta.to_string())).await;
                        }
                    }
                }
                _ => {}
            }
            if !matches!(
                event["type"].as_str(),
                Some("response.reasoning_summary_text.delta")
                    | Some("response.reasoning.summary_text.delta")
            ) {
                self.acc.handle_event(&event);
            }
            if event["type"].as_str() == Some("response.completed") {
                self.saw_terminal = true;
                let _ = tx.send(StreamEvent::Done(self.take_response())).await;
                return SseLineOutcome::Done;
            }
        }
        SseLineOutcome::Continue
    }

    async fn on_eof(&mut self, tx: &tokio::sync::mpsc::Sender<StreamEvent>) {
        if !self.saw_terminal && !self.acc.has_observable_output() {
            let _ = tx
                .send(StreamEvent::Error(
                    "Responses stream ended without completion".to_string(),
                ))
                .await;
            return;
        }
        let _ = tx.send(StreamEvent::Done(self.take_response())).await;
    }
}

#[cfg(any(test, feature = "test-support"))]
#[path = "codex_test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "codex_effort_1066_tests.rs"]
mod effort_1066_tests;

#[cfg(test)]
#[path = "codex_cov_tests.rs"]
mod cov_tests;
