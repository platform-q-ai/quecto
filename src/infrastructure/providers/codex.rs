// ChatGPT Codex adapter: impl LlmProvider using the Responses API.
//
// Used for OAuth tokens obtained via `auth.openai.com`. These tokens
// only work against `chatgpt.com/backend-api/codex/responses`, using
// the Responses API format (not Chat Completions).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, Role, ToolCall, UsageInfo};
use crate::domain::provider::{ChatRequest, LlmProvider, StreamEvent};

/// Default Codex backend base URL for ChatGPT OAuth tokens.
const CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api";

/// ChatGPT Codex provider using the Responses API.
#[derive(Debug)]
pub struct CodexProvider {
    api_key: String,
    api_base: String,
    client: reqwest::Client,
    account_id: String,
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
            account_id,
        }
    }

    /// Build Codex-specific request headers.
    fn apply_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("chatgpt-account-id", &self.account_id)
            .header("OpenAI-Beta", "responses=experimental")
            .header("originator", "codex_cli_rs")
            .header("accept", "text/event-stream")
    }

    /// Convert our domain messages into Responses API `input` array.
    ///
    /// Calls [`crate::domain::session::filter_orphan_tool_pairs`] to exclude
    /// mismatched function_call/function_call_output pairs (which would cause
    /// HTTP 400). Logs any orphaned pairs with Codex-specific context.
    fn build_input(messages: &[Message]) -> (Option<String>, Vec<serde_json::Value>) {
        let (valid_pairs, diag) = crate::domain::session::filter_orphan_tool_pairs(messages);
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

        for msg in messages {
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
                                "content": msg.content,
                            }));
                        }
                    } else {
                        input.push(serde_json::json!({
                            "role": "assistant",
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
    fn validate_request(request: &ChatRequest<'_>) -> Result<(), DomainError> {
        if request.model.contains('/') {
            return Err(DomainError::Provider(
                "codex provider expects a bare model id (e.g. 'gpt-5.3-codex'), not a provider-qualified name".to_string(),
            ));
        }

        let has_instructions = request
            .messages
            .iter()
            .any(|m| matches!(m.role, Role::System) && !m.content.trim().is_empty());
        if !has_instructions {
            return Err(DomainError::Provider(
                "codex provider requires instructions; include a non-empty system message (e.g. pass --system)".to_string(),
            ));
        }

        Ok(())
    }

    /// Build the full request body.
    fn build_request_body(request: &ChatRequest<'_>) -> serde_json::Value {
        let (instructions, input) = Self::build_input(request.messages);

        let mut body = serde_json::json!({
            "model": request.model,
            "input": input,
            "store": false,
            "stream": true,
            "tool_choice": "auto",
            "parallel_tool_calls": true,
            "reasoning": {
                "effort": "medium",
                "summary": "auto",
            },
            "text": {
                "verbosity": "medium",
            },
            "include": ["reasoning.encrypted_content"],
        });

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

    /// Parse a non-streaming Responses API response.
    #[cfg(test)]
    fn parse_response(body: &serde_json::Value) -> Result<LlmResponse, DomainError> {
        let output = body["output"]
            .as_array()
            .ok_or_else(|| DomainError::Provider("missing output in response".into()))?;

        let mut content: Option<String> = None;
        let mut tool_calls = Vec::new();

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
                    tool_calls.push(ToolCall {
                        id: call_id,
                        name,
                        arguments,
                    });
                }
                _ => {} // Skip reasoning, etc.
            }
        }

        let usage = body["usage"].as_object().map(|u| UsageInfo {
            prompt_tokens: u["input_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
            cache_read_tokens: None,
            cache_write_tokens: None,
            cost: None,
        });

        Ok(LlmResponse {
            content,
            tool_calls,
            usage,
            stop_reason: None,
            thinking_blocks: vec![],
        })
    }

    /// Parse SSE stream from the Responses API and assemble a complete response.
    fn parse_sse_response(raw: &str) -> Result<LlmResponse, DomainError> {
        let mut acc = SseAccumulator::default();

        for line in raw.lines() {
            let line = line.trim();
            if !line.starts_with("data: ") {
                continue;
            }
            let data = &line[6..];
            if data == "[DONE]" {
                break;
            }
            if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
                acc.handle_event(&event);
            }
        }

        Ok(acc.into_response())
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

    /// Public accessor for `build_request_body` (for BDD/integration tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn build_request_body_public(request: &ChatRequest<'_>) -> serde_json::Value {
        Self::build_request_body(request)
    }

    /// Public accessor for `build_input` (for BDD/integration tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn build_input_public(messages: &[Message]) -> (Option<String>, Vec<serde_json::Value>) {
        Self::build_input(messages)
    }

    /// Public accessor for `sanitize_cache_key` (for tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn sanitize_cache_key_public(key: &str) -> String {
        Self::sanitize_cache_key(key)
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
                    .send(StreamEvent::Error(format!("Codex request failed: {e}")))
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

    /// Public accessor for `parse_sse_response` (for BDD/integration tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn parse_sse_response_public(raw: &str) -> Result<LlmResponse, DomainError> {
        Self::parse_sse_response(raw)
    }
}

/// Accumulator for assembling Responses API SSE events into a response.
///
/// The Responses API emits `output_index` values that reflect the position
/// of each item in the full output array, which may include reasoning items
/// that are not tracked in our dense `tool_calls` vector. We maintain a
/// `HashMap<usize, usize>` mapping `output_index → tool_calls index` so
/// that `response.function_call_arguments.delta` events are routed to the
/// correct tool call regardless of intervening non-tool output items.
#[derive(Default)]
struct SseAccumulator {
    content: String,
    tool_calls: Vec<ToolCall>,
    /// Maps SSE `output_index` to the index in `tool_calls`.
    output_index_to_tool: HashMap<usize, usize>,
    usage: Option<UsageInfo>,
}

impl SseAccumulator {
    fn handle_event(&mut self, event: &serde_json::Value) {
        match event["type"].as_str() {
            Some("response.output_text.delta") => {
                if let Some(delta) = event["delta"].as_str() {
                    self.content.push_str(delta);
                }
            }
            Some("response.output_item.added") => self.handle_item_added(event),
            Some("response.function_call_arguments.delta") => {
                if let Some(delta) = event["delta"].as_str() {
                    let output_idx = event["output_index"].as_u64().unwrap_or(0) as usize;
                    if let Some(&tc_idx) = self.output_index_to_tool.get(&output_idx) {
                        if let Some(tc) = self.tool_calls.get_mut(tc_idx) {
                            tc.arguments.push_str(delta);
                        }
                    }
                }
            }
            Some("response.completed") => {
                if let Some(resp) = event.get("response") {
                    self.usage = resp["usage"].as_object().map(|u| UsageInfo {
                        prompt_tokens: u["input_tokens"].as_u64().unwrap_or(0) as u32,
                        completion_tokens: u["output_tokens"].as_u64().unwrap_or(0) as u32,
                        cache_read_tokens: None,
                        cache_write_tokens: None,
                        cost: None,
                    });
                }
            }
            _ => {}
        }
    }

    fn handle_item_added(&mut self, event: &serde_json::Value) {
        if let Some(item) = event.get("item") {
            if item["type"].as_str() == Some("function_call") {
                let output_idx = event["output_index"].as_u64().unwrap_or(0) as usize;
                let tc_idx = self.tool_calls.len();
                self.output_index_to_tool.insert(output_idx, tc_idx);
                self.tool_calls.push(ToolCall {
                    id: item["call_id"].as_str().unwrap_or_default().to_string(),
                    name: item["name"].as_str().unwrap_or_default().to_string(),
                    arguments: String::new(),
                });
            }
        }
    }

    fn into_response(self) -> LlmResponse {
        LlmResponse {
            content: if self.content.is_empty() {
                None
            } else {
                Some(self.content)
            },
            tool_calls: self.tool_calls,
            usage: self.usage,
            stop_reason: None,
            thinking_blocks: vec![],
        }
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
        if let Err(err) = Self::validate_request(&request) {
            return Box::pin(async move { Err(err) });
        }

        let body = Self::build_request_body(&request);
        let url = format!("{}/codex/responses", self.api_base);

        Box::pin(async move {
            let resp = self
                .apply_headers(self.client.post(&url))
                .json(&body)
                .send()
                .await
                .map_err(|e| DomainError::Provider(format!("Codex request failed: {}", e)))?;

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
        if let Err(err) = Self::validate_request(&request) {
            return Box::pin(async move {
                let (tx, rx) = tokio::sync::mpsc::channel(1);
                let _ = tx.send(StreamEvent::Error(err.to_string())).await;
                rx
            });
        }
        let body = Self::build_request_body(&request);
        let url = format!("{}/codex/responses", self.api_base);
        let api_key = self.api_key.clone();
        let api_base = self.api_base.clone();
        let account_id = self.account_id.clone();
        let client = self.client.clone();
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(64);
            tokio::spawn(async move {
                let provider = CodexProvider {
                    api_key,
                    api_base,
                    client,
                    account_id,
                };
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
}

impl CodexSseHandler {
    fn new() -> Self {
        Self {
            acc: SseAccumulator::default(),
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
            let _ = tx.send(StreamEvent::Done(self.take_response())).await;
            return SseLineOutcome::Done;
        }
        if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
            if event["type"].as_str() == Some("response.output_text.delta") {
                if let Some(delta) = event["delta"].as_str() {
                    let _ = tx.send(StreamEvent::TextDelta(delta.to_string())).await;
                }
            }
            self.acc.handle_event(&event);
            if event["type"].as_str() == Some("response.completed") {
                let _ = tx.send(StreamEvent::Done(self.take_response())).await;
                return SseLineOutcome::Done;
            }
        }
        SseLineOutcome::Continue
    }

    async fn on_eof(&mut self, tx: &tokio::sync::mpsc::Sender<StreamEvent>) {
        let _ = tx.send(StreamEvent::Done(self.take_response())).await;
    }
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
