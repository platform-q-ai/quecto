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

use crate::application::providers::ports::{ChatRequest, LlmProvider};
#[cfg(any(test, feature = "test-support"))]
use crate::domain::conversation::value_objects::message::Message;
use crate::domain::conversation::value_objects::message::{LlmResponse, Role, ThinkingBlock};
use crate::domain::error::DomainError;
use crate::domain::inference::value_objects::provider::StreamEvent;

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
    attempt_admission: Option<std::sync::Arc<dyn crate::application::ports::AttemptAdmission>>,
    auth: ResponsesAuth,
    /// Set once the service refused replayed reasoning: this provider (and
    /// its clones) replays no more (#2162 review).
    replay_refused: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// A random identity minted when an API-key provider is built: the
    /// origin its reasoning replays to. Nothing is derived from the key,
    /// which must never reach a session file, not even as a digest (#2162
    /// swarm review). Empty for ChatGPT OAuth, whose account identifies it.
    key_origin: String,
    /// The bound on a silent streaming reply (#2210).
    stream_idle: super::stream_idle::StreamIdle,
}

impl CodexProvider {
    /// Bound its replies by `stream_idle`: a provider's configured stream
    /// idle limit (#2433 review), or the defaults.
    pub fn with_stream_idle(mut self, stream_idle: super::stream_idle::StreamIdle) -> Self {
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
            attempt_admission: None,
            replay_refused: Default::default(),
            key_origin: String::new(),
            stream_idle: Default::default(),
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
            attempt_admission: None,
            replay_refused: Default::default(),
            key_origin: uuid::Uuid::new_v4().to_string(),
            stream_idle: Default::default(),
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

    /// Build request headers for the active auth mode. A ChatGPT OAuth
    /// request also names its session, as the official client does
    /// (#2162): the same sanitised key as `prompt_cache_key`.
    fn apply_headers(
        &self,
        builder: reqwest::RequestBuilder,
        session: Option<&str>,
    ) -> reqwest::RequestBuilder {
        let builder = builder
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("accept", "text/event-stream");
        match &self.auth {
            ResponsesAuth::ChatGptOAuth { account_id } => {
                let builder = builder
                    .header("chatgpt-account-id", account_id)
                    .header("OpenAI-Beta", "responses=experimental")
                    .header("originator", "codex_cli_rs");
                match session {
                    Some(session) => builder.header("session_id", session),
                    None => builder,
                }
            }
            ResponsesAuth::ApiKey => builder,
        }
    }

    /// Cost the response, and stamp its reasoning items with the origin
    /// they are replayed to (#2162).
    pub(super) fn finish_response(response: &mut LlmResponse, model: &str, origin: &str) {
        crate::domain::inference::services::usage_accounting::attach_cost(response, model);
        for block in &mut response.thinking_blocks {
            if let ThinkingBlock::EncryptedReasoning {
                origin: stamped, ..
            } = block
            {
                *stamped = origin.to_string();
            }
        }
    }

    /// Where a request's reasoning can be decrypted again (#2162 review):
    /// the endpoint, who asked, and the model. A ChatGPT account is named by
    /// a digest of its id (sent openly as a header, not a secret), so a
    /// resumed session replays. An API-key provider is named by the random
    /// identity it was built with — never by anything derived from the key
    /// (#2162 swarm review) — so its reasoning replays only while it lives.
    fn reasoning_origin(&self, model: &str) -> String {
        let account = match &self.auth {
            ResponsesAuth::ChatGptOAuth { account_id } => format!("{:08x}", fnv1a(account_id)),
            ResponsesAuth::ApiKey => format!("key-{}", self.key_origin),
        };
        format!("{}|{account}|{model}", self.responses_url())
    }

    /// The session a request names, sanitised as its cache key is.
    fn request_session(request: &ChatRequest<'_>) -> Option<String> {
        request.session_id.map(Self::sanitize_cache_key)
    }

    /// Build the Responses API tool definitions.
    fn build_tools(
        tools: &[crate::domain::tool_policy::value_objects::tool::ToolDefinition],
    ) -> Vec<serde_json::Value> {
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
                "codex provider expects a bare model id (e.g. 'gpt-6.1-sol'), not a provider-qualified name".to_string(),
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
    fn build_request_body(
        request: &ChatRequest<'_>,
        auth: &ResponsesAuth,
        origin: &str,
    ) -> serde_json::Value {
        let (instructions, input) = Self::build_input_for(request.messages, origin);

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
    fn reasoning_effort_str(
        effort: crate::domain::inference::value_objects::provider::EffortLevel,
    ) -> &'static str {
        use crate::domain::inference::value_objects::provider::EffortLevel;
        match effort {
            EffortLevel::Max => "xhigh",
            other => other.as_str(),
        }
    }

    /// Map an effort level onto the Responses API `text.verbosity` scale,
    /// which only accepts `low`/`medium`/`high`: levels outside that scale
    /// clamp to the nearest documented value.
    fn verbosity_str(
        effort: crate::domain::inference::value_objects::provider::EffortLevel,
    ) -> &'static str {
        use crate::domain::inference::value_objects::provider::EffortLevel;
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
                    let arguments = match &item["arguments"] {
                        serde_json::Value::String(text) => text.clone(),
                        serde_json::Value::Object(_) => item["arguments"].to_string(),
                        _ => String::new(),
                    };
                    tool_calls.push(
                        crate::domain::conversation::value_objects::message::ToolCall {
                            id: call_id,
                            name,
                            arguments,
                        },
                    );
                }
                Some("reasoning") => {
                    codex_sse_state::append_reasoning_summary(item, &mut reasoning)?;
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
            vec![
                crate::domain::conversation::value_objects::message::ThinkingBlock::Normal {
                    thinking: reasoning,
                    signature: String::new(),
                },
            ]
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
    #[cfg(any(test, feature = "test-support"))]
    fn parse_sse_response(raw: &str) -> Result<LlmResponse, DomainError> {
        Self::parse_sse_reply(raw).map_err(super::sse_end::UnfinishedReply::into_error)
    }

    /// [`Self::parse_sse_response`], keeping the usage a reply cut short
    /// before its terminal event reported, to be accounted (#2249 review).
    fn parse_sse_reply(raw: &str) -> Result<LlmResponse, super::sse_end::UnfinishedReply> {
        let mut acc = SseAccumulator::default();
        let mut saw_terminal = false;
        let mut saw_event = false;

        for line in raw.lines() {
            // Only line ends are trimmed: an indented line is no event.
            let line = line.trim_end();
            let Some(data) = super::sse_common::event_data(line) else {
                continue;
            };
            saw_event = true;
            if super::sse_end::is_done_marker(data) {
                saw_terminal = true;
                break;
            }
            if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
                if let Some(error) = Self::format_stream_failure(&event) {
                    return Err(DomainError::Provider(error).into());
                }
                acc.handle_event(&event)?;
                if event["type"].as_str() == Some("response.completed") {
                    saw_terminal = true;
                    break;
                }
            }
        }

        // Only a terminal event ends a reply whole: output without one is a
        // reply cut short, and no event at all an empty stream (#2249
        // review), never taken as a whole answer.
        if !saw_terminal {
            return Err(super::sse_end::UnfinishedReply {
                error: DomainError::Provider(super::sse_end::ended_early(
                    saw_event,
                    RESPONSES_CUT_SHORT,
                )),
                usage: acc.into_response().usage,
            });
        }

        Ok(acc.into_response())
    }

    /// The error a Responses event ends the stream with, if it is one: a
    /// typed failure (`response.failed`, `response.incomplete`, `error`)
    /// or an untyped error chunk (#2249 review), which reads as `error`.
    /// An `error` event carries its fields nested (`error.type`,
    /// `error.code`, `error.message`) or, as OpenAI documents it, at the
    /// top level (`code`, `message`); both are read. A `response.*`
    /// failure keeps its established rendering (type and message).
    fn format_stream_failure(event: &serde_json::Value) -> Option<String> {
        let kind = match event["type"].as_str() {
            Some(kind @ ("response.failed" | "response.incomplete" | "error")) => kind,
            None if super::attempt_profile::is_untyped_error_chunk(event) => "error",
            _ => return None,
        };
        let mut parts = vec![format!("Responses stream {kind}")];
        if let Some(status) = event["response"]["status"].as_str() {
            parts.push(format!("status={status}"));
        }
        if let Some(reason) = event["response"]["incomplete_details"]["reason"].as_str() {
            parts.push(format!("reason={reason}"));
        }
        // An `error` event's own top-level fields; no other kind has any.
        let top = match kind {
            "error" => event,
            _ => &serde_json::Value::Null,
        };
        let error = match kind {
            "error" => &event["error"],
            _ => &event["response"]["error"],
        };
        if let Some(error_type) = error["type"].as_str() {
            parts.push(format!("type={error_type}"));
        }
        if kind == "error" {
            if let Some(code) = error["code"].as_str().or_else(|| top["code"].as_str()) {
                parts.push(format!("code={code}"));
            }
        }
        // A bare string `error` (#2236) is the message itself.
        if let Some(message) = error
            .as_str()
            .or_else(|| error["message"].as_str())
            .or_else(|| top["message"].as_str())
        {
            parts.push(message.to_string());
        }
        Some(parts.join(": "))
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
    ///
    /// The prefix is kept only when it is plain (`[A-Za-z0-9_-]`, at most
    /// 64 bytes, which every key the harness makes fits, so none changes),
    /// so the key is always a valid `session_id` header value (#2162
    /// review); any other prefix becomes `session`.
    fn sanitize_cache_key(key: &str) -> String {
        let hash = fnv1a(key);
        let prefix = key
            .split(':')
            .next()
            .filter(|prefix| {
                (1..=64).contains(&prefix.len())
                    && prefix
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            })
            .unwrap_or("session");
        format!("{prefix}:{hash:08x}")
    }

    /// Consume SSE body incrementally, emitting `StreamEvent`s per delta;
    /// observed beside the request when it carries a trace (#2151, #2210).
    async fn pump_codex_sse(
        &self,
        call: &codex_replay::Call,
        body: serde_json::Value,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
        handler: CodexSseHandler,
    ) {
        let idle = self.stream_idle;
        let attempt = super::attempt_transport::PassiveAttempt::begin(
            call.trace.clone(),
            super::attempt_profile::Profile::new(
                super::attempt_profile::Vendor::Codex,
                super::attempt_profile::Surface::Incremental,
                idle,
            ),
        );
        let builder = self
            .apply_headers(self.client.post(&call.url), call.session.as_deref())
            .json(&body);
        let mut response = match idle.within(builder.send()).await {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                attempt.iter().for_each(|a| a.send_failed());
                let _ = tx
                    .send(StreamEvent::Error(format!(
                        "Codex request failed: {}",
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
            let read = idle.text(response).await;
            attempt.iter().for_each(|a| a.error_read(status, &read));
            let text = super::sse_common::truncate_error_body(super::stream_idle::error_text(read));
            let _ = tx
                .send(StreamEvent::Error(format!(
                    "HTTP {status} from Codex: {text}"
                )))
                .await;
            return;
        }
        super::attempt_transport::pump_observed(&mut response, &tx, handler, attempt, idle).await;
    }

    #[cfg(test)]
    pub(crate) async fn pump_sse_response_for_test(
        mut response: reqwest::Response,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) {
        let mut handler = CodexSseHandler::new();
        super::sse_common::pump_sse(&mut response, &tx, &mut handler, Default::default()).await;
    }

    /// Public accessor for `parse_sse_response` (for BDD/integration tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn parse_sse_response_public(raw: &str) -> Result<LlmResponse, DomainError> {
        Self::parse_sse_response(raw)
    }
}

impl LlmProvider for CodexProvider {
    fn route_order(&self) -> Vec<String> {
        vec![self.name().to_string()]
    }
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

        let (call, bodies) = self.prepare(&request);
        Box::pin(async move { self.assemble(&call, bodies).await })
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
        let (call, bodies) = self.prepare(&request);
        let provider = self.clone();
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(64);
            super::attempt_transport::queue_before_spawn(
                provider.attempt_admission.as_ref(),
                call.trace.as_ref(),
            );
            tokio::spawn(provider.stream(call, bodies, tx));
            rx
        })
    }
}

#[path = "codex_input.rs"]
mod codex_input;
#[path = "codex_replay.rs"]
mod codex_replay;
/// FNV-1a 32-bit: fast, dependency-free, deterministic.
fn fnv1a(text: &str) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in text.bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

#[path = "codex_sse_handler.rs"]
mod codex_sse_handler;
#[cfg(test)]
use super::sse_common::{SseHandler, SseLineOutcome};
use codex_sse_handler::CodexSseHandler;
pub(crate) use codex_sse_handler::RESPONSES_CUT_SHORT;

#[cfg(any(test, feature = "test-support"))]
#[path = "codex_test_support.rs"]
mod test_support;

#[cfg(test)]
#[path = "codex_2434_tests.rs"]
mod empty_reply_2434_tests;
#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "codex_args_tests.rs"]
mod args_tests;
#[cfg(test)]
#[path = "codex_stop_reason_tests.rs"]
mod stop_reason_tests;

#[cfg(test)]
#[path = "codex_effort_1066_tests.rs"]
mod effort_1066_tests;

#[cfg(test)]
#[path = "codex_cov_tests.rs"]
mod cov_tests;

#[cfg(test)]
#[path = "codex_2162_tests.rs"]
mod issue_2162_tests;

#[cfg(test)]
#[path = "codex_phase_2397_tests.rs"]
mod phase_2397_tests;

#[cfg(test)]
#[path = "codex_stream_end_tests.rs"]
mod stream_end_tests;

#[cfg(test)]
#[path = "codex_2398_tests.rs"]
mod issue_2398_tests;

#[cfg(test)]
#[path = "codex_2421_tests.rs"]
mod issue_2421_tests;
