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
use crate::domain::provider::{ChatRequest, LlmProvider};

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
        Self {
            api_key,
            api_base: api_base.unwrap_or_else(|| CODEX_BASE_URL.to_string()),
            client: reqwest::Client::new(),
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
    fn build_input(messages: &[Message]) -> (Option<String>, Vec<serde_json::Value>) {
        let mut instructions: Option<String> = None;
        let mut input = Vec::new();

        for msg in messages {
            match msg.role {
                Role::System => {
                    // Collect system messages as instructions
                    match &mut instructions {
                        Some(existing) => {
                            existing.push('\n');
                            existing.push_str(&msg.content);
                        }
                        None => instructions = Some(msg.content.clone()),
                    }
                }
                Role::User => {
                    input.push(serde_json::json!({
                        "role": "user",
                        "content": msg.content,
                    }));
                }
                Role::Assistant => {
                    if !msg.tool_calls.is_empty() {
                        // Emit function_call items for each tool call
                        for tc in &msg.tool_calls {
                            input.push(serde_json::json!({
                                "type": "function_call",
                                "call_id": tc.id,
                                "name": tc.name,
                                "arguments": tc.arguments,
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
                    // Tool results become function_call_output items
                    if let Some(ref call_id) = msg.tool_call_id {
                        input.push(serde_json::json!({
                            "type": "function_call_output",
                            "call_id": call_id,
                            "output": msg.content,
                        }));
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

        if let Some(ref session_id) = request.session_id {
            body["prompt_cache_key"] = serde_json::Value::String(session_id.clone());
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
        });

        Ok(LlmResponse {
            content,
            tool_calls,
            usage,
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

    /// Public accessor for `build_request_body` (for BDD/integration tests).
    #[cfg(any(test, feature = "test-support"))]
    pub fn build_request_body_public(request: &ChatRequest<'_>) -> serde_json::Value {
        Self::build_request_body(request)
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

    fn chat_stream(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        self.chat(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::tool::ToolDefinition;

    #[test]
    fn test_build_input_basic_messages() {
        let messages = vec![
            Message::system("You are helpful."),
            Message::user("Hello"),
            Message::assistant("Hi there!", vec![]),
        ];
        let (instructions, input) = CodexProvider::build_input(&messages);
        assert_eq!(instructions.unwrap(), "You are helpful.");
        assert_eq!(input.len(), 2);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"], "Hello");
        assert_eq!(input[1]["role"], "assistant");
        assert_eq!(input[1]["content"], "Hi there!");
    }

    #[test]
    fn test_build_input_tool_calls() {
        let mut assistant_msg = Message::assistant("", vec![]);
        assistant_msg.tool_calls = vec![ToolCall {
            id: "call_123".to_string(),
            name: "get_weather".to_string(),
            arguments: r#"{"location":"Paris"}"#.to_string(),
        }];

        let tool_msg = Message::tool("call_123", "sunny");

        let messages = vec![Message::user("Weather?"), assistant_msg, tool_msg];
        let (instructions, input) = CodexProvider::build_input(&messages);
        assert!(instructions.is_none());
        assert_eq!(input.len(), 3);
        // User message
        assert_eq!(input[0]["role"], "user");
        // Function call
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["call_id"], "call_123");
        assert_eq!(input[1]["name"], "get_weather");
        // Function call output
        assert_eq!(input[2]["type"], "function_call_output");
        assert_eq!(input[2]["call_id"], "call_123");
        assert_eq!(input[2]["output"], "sunny");
    }

    #[test]
    fn test_build_tools() {
        let tools = vec![ToolDefinition {
            name: "read".to_string(),
            description: "Read a file".to_string(),
            parameters_schema: r#"{"type":"object","properties":{"path":{"type":"string"}}}"#
                .to_string(),
        }];
        let result = CodexProvider::build_tools(&tools);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["type"], "function");
        assert_eq!(result[0]["name"], "read");
        assert!(result[0]["parameters"]["properties"]["path"].is_object());
    }

    #[test]
    fn test_build_request_body() {
        let messages = vec![Message::system("Be concise."), Message::user("Hi")];
        let tools = vec![];
        let request = ChatRequest {
            messages: &messages,
            tools: &tools,
            model: "gpt-5.1-codex",
            max_tokens: 4096,
            temperature: 0.7,
            session_id: None,
        };
        let body = CodexProvider::build_request_body(&request);
        assert_eq!(body["model"], "gpt-5.1-codex");
        assert_eq!(body["instructions"], "Be concise.");
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert_eq!(body["input"].as_array().unwrap().len(), 1);
        assert!(body.get("tools").is_none());
    }

    // --- prompt_cache_key ---

    #[test]
    fn test_build_request_body_includes_prompt_cache_key_when_session_id_set() {
        let messages = vec![Message::user("Hi")];
        let request = ChatRequest {
            messages: &messages,
            tools: &[],
            model: "gpt-5.3-codex",
            max_tokens: 4096,
            temperature: 0.7,
            session_id: Some("cli:default".to_string()),
        };
        let body = CodexProvider::build_request_body(&request);
        assert_eq!(
            body["prompt_cache_key"].as_str(),
            Some("cli:default"),
            "expected prompt_cache_key to be 'cli:default', got: {:?}",
            body.get("prompt_cache_key")
        );
    }

    #[test]
    fn test_build_request_body_omits_prompt_cache_key_when_no_session_id() {
        let messages = vec![Message::user("Hi")];
        let request = ChatRequest {
            messages: &messages,
            tools: &[],
            model: "gpt-5.3-codex",
            max_tokens: 4096,
            temperature: 0.7,
            session_id: None,
        };
        let body = CodexProvider::build_request_body(&request);
        assert!(
            body.get("prompt_cache_key").is_none(),
            "expected prompt_cache_key to be absent, got: {:?}",
            body.get("prompt_cache_key")
        );
    }

    #[test]
    fn test_build_request_body_responses_api_fields() {
        let messages = vec![Message::user("Hi")];
        let tools = vec![];
        let request = ChatRequest {
            messages: &messages,
            tools: &tools,
            model: "gpt-5.3-codex",
            max_tokens: 4096,
            temperature: 0.7,
            session_id: None,
        };
        let body = CodexProvider::build_request_body(&request);
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["parallel_tool_calls"], true);
        assert_eq!(body["reasoning"]["effort"], "medium");
        assert_eq!(body["reasoning"]["summary"], "auto");
        assert_eq!(body["text"]["verbosity"], "medium");
        let include = body["include"].as_array().unwrap();
        assert!(
            include
                .iter()
                .any(|v| v.as_str() == Some("reasoning.encrypted_content"))
        );
        assert!(body.get("max_completion_tokens").is_none());
    }

    #[test]
    fn test_build_tools_includes_strict_false() {
        let tools = vec![ToolDefinition {
            name: "exec".to_string(),
            description: "Execute a command".to_string(),
            parameters_schema: r#"{"type":"object","properties":{"command":{"type":"string"}}}"#
                .to_string(),
        }];
        let result = CodexProvider::build_tools(&tools);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["strict"], false);
    }

    #[test]
    fn test_parse_sse_tool_call_after_reasoning_item() {
        // Reasoning item at output_index 0, function_call at output_index 1
        let sse = r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning"}}
data: {"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","call_id":"call_x","name":"exec","arguments":""}}
data: {"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"cmd\""}
data: {"type":"response.function_call_arguments.delta","output_index":1,"delta":":\"ls\"}"}
data: {"type":"response.completed","response":{"usage":{"input_tokens":5,"output_tokens":3}}}
data: [DONE]
"#;
        let resp = CodexProvider::parse_sse_response(sse).unwrap();
        assert!(resp.content.is_none());
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].id, "call_x");
        assert_eq!(resp.tool_calls[0].name, "exec");
        assert_eq!(resp.tool_calls[0].arguments, r#"{"cmd":"ls"}"#);
    }

    #[test]
    fn test_parse_sse_multiple_tool_calls_after_reasoning() {
        let sse = r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning"}}
data: {"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","call_id":"c1","name":"read","arguments":""}}
data: {"type":"response.output_item.added","output_index":2,"item":{"type":"function_call","call_id":"c2","name":"write","arguments":""}}
data: {"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"path\":\"a.rs\"}"}
data: {"type":"response.function_call_arguments.delta","output_index":2,"delta":"{\"content\":\"hi\"}"}
data: [DONE]
"#;
        let resp = CodexProvider::parse_sse_response(sse).unwrap();
        assert_eq!(resp.tool_calls.len(), 2);
        assert_eq!(resp.tool_calls[0].name, "read");
        assert_eq!(resp.tool_calls[0].arguments, r#"{"path":"a.rs"}"#);
        assert_eq!(resp.tool_calls[1].name, "write");
        assert_eq!(resp.tool_calls[1].arguments, r#"{"content":"hi"}"#);
    }

    #[test]
    fn test_parse_response_text() {
        let body = serde_json::json!({
            "output": [
                {
                    "type": "message",
                    "content": [
                        { "type": "output_text", "text": "Hello!" }
                    ]
                }
            ],
            "usage": {
                "input_tokens": 10,
                "output_tokens": 5,
            }
        });
        let resp = CodexProvider::parse_response(&body).unwrap();
        assert_eq!(resp.content.unwrap(), "Hello!");
        assert!(resp.tool_calls.is_empty());
        let usage = resp.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 5);
    }

    #[test]
    fn test_parse_response_tool_call() {
        let body = serde_json::json!({
            "output": [
                {
                    "type": "function_call",
                    "call_id": "call_abc",
                    "name": "shell",
                    "arguments": "{\"command\":\"ls\"}"
                }
            ]
        });
        let resp = CodexProvider::parse_response(&body).unwrap();
        assert!(resp.content.is_none());
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].id, "call_abc");
        assert_eq!(resp.tool_calls[0].name, "shell");
        assert_eq!(resp.tool_calls[0].arguments, r#"{"command":"ls"}"#);
    }

    #[test]
    fn test_parse_sse_text_response() {
        let sse = r#"data: {"type":"response.output_text.delta","delta":"Hello"}
data: {"type":"response.output_text.delta","delta":" world"}
data: {"type":"response.completed","response":{"usage":{"input_tokens":8,"output_tokens":2}}}
data: [DONE]
"#;
        let resp = CodexProvider::parse_sse_response(sse).unwrap();
        assert_eq!(resp.content.unwrap(), "Hello world");
        let usage = resp.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 8);
        assert_eq!(usage.completion_tokens, 2);
    }

    #[test]
    fn test_parse_sse_tool_call() {
        let sse = r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"call_x","name":"shell","arguments":""}}
data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"cmd\""}
data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":":\"ls\"}"}
data: {"type":"response.function_call_arguments.done","output_index":0,"arguments":"{\"cmd\":\"ls\"}"}
data: {"type":"response.completed","response":{"usage":{"input_tokens":5,"output_tokens":3}}}
data: [DONE]
"#;
        let resp = CodexProvider::parse_sse_response(sse).unwrap();
        assert!(resp.content.is_none());
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].id, "call_x");
        assert_eq!(resp.tool_calls[0].name, "shell");
        assert_eq!(resp.tool_calls[0].arguments, r#"{"cmd":"ls"}"#);
    }

    #[tokio::test]
    async fn test_codex_provider_http_error() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(401).set_body_string("unauthorized"))
            .mount(&server)
            .await;

        let provider = CodexProvider::new(
            "test-token".to_string(),
            "acct-123".to_string(),
            Some(server.uri()),
        );
        let messages = vec![Message::user("hi")];
        let result = provider
            .chat(ChatRequest {
                messages: &messages,
                tools: &[],
                model: "gpt-5.1-codex",
                max_tokens: 1024,
                temperature: 0.7,
                session_id: None,
            })
            .await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("401"), "expected 401 in error: {}", err);
    }

    #[tokio::test]
    async fn test_codex_provider_success() {
        let server = wiremock::MockServer::start().await;
        let sse_body = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi!\"}\n\
                         data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":1}}}\n\
                         data: [DONE]\n";
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(sse_body))
            .mount(&server)
            .await;

        let provider = CodexProvider::new(
            "test-token".to_string(),
            "acct-123".to_string(),
            Some(server.uri()),
        );
        let messages = vec![Message::user("hello")];
        let result = provider
            .chat(ChatRequest {
                messages: &messages,
                tools: &[],
                model: "gpt-5.1-codex",
                max_tokens: 1024,
                temperature: 0.7,
                session_id: None,
            })
            .await;
        let resp = result.unwrap();
        assert_eq!(resp.content.unwrap(), "Hi!");
    }
}
