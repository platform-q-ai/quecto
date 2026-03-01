// Anthropic adapter: impl LlmProvider for AnthropicProvider.

use std::future::Future;
use std::pin::Pin;

use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message, Role, ToolCall, UsageInfo};
use crate::domain::provider::{ChatRequest, LlmProvider};

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
        let is_oauth = crate::infrastructure::auth::oauth::is_anthropic_oauth_token(&api_key);
        Self {
            api_key,
            api_base: api_base.unwrap_or_else(|| "https://api.anthropic.com".to_string()),
            client: reqwest::Client::new(),
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
            builder.header("x-api-key", &self.api_key)
        }
    }

    /// Build the JSON request body for Anthropic Messages API.
    fn build_request_body(request: &ChatRequest<'_>) -> (Option<String>, serde_json::Value) {
        let (system_prompt, api_messages) = Self::build_messages(request.messages);
        let mut body = serde_json::json!({
            "model": request.model,
            "messages": api_messages,
            "max_tokens": request.max_tokens,
            "temperature": request.temperature,
        });
        if let Some(ref sys) = system_prompt {
            body["system"] = serde_json::Value::String(sys.clone());
        }
        if !request.tools.is_empty() {
            body["tools"] = serde_json::Value::Array(Self::build_tool_defs(request.tools));
        }
        (system_prompt, body)
    }

    /// Convert domain messages to Anthropic API message format.
    fn build_messages(messages: &[Message]) -> (Option<String>, Vec<serde_json::Value>) {
        let mut system_prompt: Option<String> = None;
        let mut api_messages: Vec<serde_json::Value> = Vec::new();
        for m in messages {
            match m.role {
                Role::System => {
                    system_prompt = Some(m.content.clone());
                }
                Role::User => {
                    api_messages.push(serde_json::json!({"role": "user", "content": m.content}));
                }
                Role::Assistant => {
                    api_messages.push(Self::build_assistant_message(m));
                }
                Role::Tool => {
                    api_messages.push(Self::build_tool_result_message(m));
                }
            }
        }
        (system_prompt, api_messages)
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
            let input: serde_json::Value = serde_json::from_str(&tc.arguments).unwrap_or_default();
            content_blocks.push(serde_json::json!({
                "type": "tool_use",
                "id": tc.id,
                "name": tc.name,
                "input": input,
            }));
        }
        serde_json::json!({"role": "assistant", "content": content_blocks})
    }

    /// Build an Anthropic tool_result message.
    ///
    /// When image blocks are present, uses the array format so Anthropic can
    /// render them. Otherwise uses a plain string (avoids wrapping overhead).
    fn build_tool_result_message(m: &Message) -> serde_json::Value {
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
            "role": "user",
            "content": [{
                "type": "tool_result",
                "tool_use_id": m.tool_call_id.as_deref().unwrap_or(""),
                "content": content_value,
            }],
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
        });

        Ok(LlmResponse {
            content,
            tool_calls,
            usage,
        })
    }
}

impl AnthropicProvider {
    /// Send a streaming chat request and assemble the response from SSE events.
    async fn stream_chat(&self, request: ChatRequest<'_>) -> Result<LlmResponse, DomainError> {
        let (_system, mut body) = Self::build_request_body(&request);
        body["stream"] = serde_json::Value::Bool(true);
        let url = format!("{}/v1/messages", self.api_base);

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
        if status != 200 {
            let text = response.text().await.unwrap_or_default();
            return Err(DomainError::Provider(format!(
                "HTTP {} from Anthropic: {}",
                status, text
            )));
        }

        let full = response
            .text()
            .await
            .map_err(|e| DomainError::Provider(format!("failed to read stream: {}", e)))?;

        Self::parse_sse_response(&full)
    }

    /// Parse Anthropic SSE events into an assembled LlmResponse.
    fn parse_sse_response(raw: &str) -> Result<LlmResponse, DomainError> {
        let mut content = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();
        let mut current_tool_id = String::new();
        let mut current_tool_name = String::new();
        let mut current_tool_input = String::new();
        let mut in_tool_input = false;

        let mut current_event = String::new();

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
                "content_block_start" => {
                    if chunk["content_block"]["type"].as_str() == Some("tool_use") {
                        current_tool_id = chunk["content_block"]["id"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string();
                        current_tool_name = chunk["content_block"]["name"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string();
                        current_tool_input.clear();
                        in_tool_input = true;
                    }
                }
                "content_block_delta" => {
                    let delta = &chunk["delta"];
                    match delta["type"].as_str() {
                        Some("text_delta") => {
                            if let Some(text) = delta["text"].as_str() {
                                content.push_str(text);
                            }
                        }
                        Some("input_json_delta") => {
                            if let Some(json) = delta["partial_json"].as_str() {
                                current_tool_input.push_str(json);
                            }
                        }
                        _ => {}
                    }
                }
                "content_block_stop" => {
                    if in_tool_input {
                        tool_calls.push(ToolCall {
                            id: std::mem::take(&mut current_tool_id),
                            name: std::mem::take(&mut current_tool_name),
                            arguments: std::mem::take(&mut current_tool_input),
                        });
                        in_tool_input = false;
                    }
                }
                "message_stop" => break,
                _ => {}
            }
        }

        let content_opt = if content.is_empty() {
            None
        } else {
            Some(content)
        };

        Ok(LlmResponse {
            content: content_opt,
            tool_calls,
            usage: None,
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
        let (_system, body) = Self::build_request_body(&request);
        let url = format!("{}/v1/messages", self.api_base);

        Box::pin(async move {
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

            Self::parse_response(&response_json)
        })
    }

    fn chat_stream(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        // Clone request data to avoid lifetime conflicts.
        let messages = request.messages.to_vec();
        let tools = request.tools.to_vec();
        let model = request.model.to_string();
        let max_tokens = request.max_tokens;
        let temperature = request.temperature;
        Box::pin(async move {
            let req = ChatRequest {
                messages: &messages,
                tools: &tools,
                model: &model,
                max_tokens,
                temperature,
            };
            self.stream_chat(req).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::message::Message;
    use crate::domain::provider::ChatRequest;
    use crate::domain::tool::ToolDefinition;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn test_anthropic_provider_name() {
        let provider = AnthropicProvider::new("sk-ant-test".to_string(), None);
        assert_eq!(provider.name(), "anthropic");
    }

    #[tokio::test]
    async fn test_chat_text_response() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "content": [{ "type": "text", "text": "Hello from Claude!" }],
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 12, "output_tokens": 8 }
        });
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .mount(&server)
            .await;

        let provider = AnthropicProvider::new("sk-ant-test".to_string(), Some(server.uri()));
        let messages = vec![Message::user("Hi")];
        let req = ChatRequest {
            messages: &messages,
            tools: &[],
            model: "claude-sonnet-4-20250514",
            max_tokens: 1024,
            temperature: 0.7,
        };
        let result = provider.chat(req).await;
        assert!(result.is_ok(), "chat should succeed: {:?}", result);
        let response = result.unwrap();
        assert_eq!(response.content.as_deref(), Some("Hello from Claude!"));
        assert!(response.tool_calls.is_empty());
        let usage = response.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 12);
        assert_eq!(usage.completion_tokens, 8);
    }

    #[tokio::test]
    async fn test_chat_with_tool_use() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "id": "msg_456",
            "type": "message",
            "role": "assistant",
            "content": [{
                "type": "tool_use",
                "id": "toolu_abc",
                "name": "exec",
                "input": { "command": "ls" }
            }],
            "stop_reason": "tool_use",
            "usage": { "input_tokens": 20, "output_tokens": 15 }
        });
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .mount(&server)
            .await;

        let provider = AnthropicProvider::new("sk-ant-test".to_string(), Some(server.uri()));
        let messages = vec![Message::user("list files")];
        let tools = vec![ToolDefinition {
            name: "exec".to_string(),
            description: "Execute a command".to_string(),
            parameters_schema: r#"{"type":"object","properties":{"command":{"type":"string"}}}"#
                .to_string(),
        }];
        let req = ChatRequest {
            messages: &messages,
            tools: &tools,
            model: "claude-sonnet-4-20250514",
            max_tokens: 1024,
            temperature: 0.7,
        };
        let result = provider.chat(req).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].id, "toolu_abc");
        assert_eq!(response.tool_calls[0].name, "exec");
        assert!(response.tool_calls[0].arguments.contains("ls"));
    }

    #[tokio::test]
    async fn test_chat_server_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
            .mount(&server)
            .await;

        let provider = AnthropicProvider::new("sk-ant-test".to_string(), Some(server.uri()));
        let messages = vec![Message::user("Hi")];
        let req = ChatRequest {
            messages: &messages,
            tools: &[],
            model: "claude-sonnet-4-20250514",
            max_tokens: 1024,
            temperature: 0.7,
        };
        let result = provider.chat(req).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("500"), "error should mention status: {}", err);
    }

    #[test]
    fn test_parse_sse_text_response() {
        let sse = "\
event: content_block_delta\n\
data: {\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n\
event: content_block_delta\n\
data: {\"delta\":{\"type\":\"text_delta\",\"text\":\" from Claude\"}}\n\n\
event: message_stop\n\
data: {}\n";
        let result = AnthropicProvider::parse_sse_response(sse).unwrap();
        assert_eq!(result.content.as_deref(), Some("Hello from Claude"));
        assert!(result.tool_calls.is_empty());
    }

    #[test]
    fn test_parse_sse_tool_use() {
        let sse = "\
event: content_block_start\n\
data: {\"content_block\":{\"type\":\"tool_use\",\"id\":\"tu_1\",\"name\":\"exec\"}}\n\n\
event: content_block_delta\n\
data: {\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"cmd\\\"\"}}\n\n\
event: content_block_delta\n\
data: {\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\": \\\"ls\\\"}\"}}\n\n\
event: content_block_stop\n\
data: {}\n\n\
event: message_stop\n\
data: {}\n";
        let result = AnthropicProvider::parse_sse_response(sse).unwrap();
        assert!(result.content.is_none());
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].id, "tu_1");
        assert_eq!(result.tool_calls[0].name, "exec");
        assert!(result.tool_calls[0].arguments.contains("ls"));
    }

    #[test]
    fn test_parse_sse_empty_stops() {
        let sse = "event: message_stop\ndata: {}\n";
        let result = AnthropicProvider::parse_sse_response(sse).unwrap();
        assert!(result.content.is_none());
        assert!(result.tool_calls.is_empty());
    }

    #[tokio::test]
    async fn test_chat_stream_with_mock() {
        let server = MockServer::start().await;
        let sse_body = "\
event: content_block_delta\n\
data: {\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n\
event: content_block_delta\n\
data: {\"delta\":{\"type\":\"text_delta\",\"text\":\" there\"}}\n\n\
event: message_stop\n\
data: {}\n\n";
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(sse_body)
                    .insert_header("content-type", "text/event-stream"),
            )
            .mount(&server)
            .await;

        let provider = AnthropicProvider::new("sk-ant-test".to_string(), Some(server.uri()));
        let messages = vec![Message::user("Hi")];
        let req = ChatRequest {
            messages: &messages,
            tools: &[],
            model: "claude-sonnet-4-20250514",
            max_tokens: 1024,
            temperature: 0.7,
        };
        let result = provider.chat_stream(req).await;
        assert!(result.is_ok(), "stream should succeed: {:?}", result);
        let resp = result.unwrap();
        assert_eq!(resp.content.as_deref(), Some("Hi there"));
    }

    #[tokio::test]
    async fn test_chat_with_system_prompt() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "id": "msg_789",
            "type": "message",
            "role": "assistant",
            "content": [{ "type": "text", "text": "I am helpful." }],
            "stop_reason": "end_turn",
            "usage": { "input_tokens": 15, "output_tokens": 5 }
        });
        Mock::given(method("POST"))
            .and(path("/v1/messages"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let provider = AnthropicProvider::new("sk-ant-test".to_string(), Some(server.uri()));
        let messages = vec![Message::system("You are helpful."), Message::user("Hi")];
        let req = ChatRequest {
            messages: &messages,
            tools: &[],
            model: "claude-sonnet-4-20250514",
            max_tokens: 1024,
            temperature: 0.7,
        };
        let result = provider.chat(req).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().content.as_deref(), Some("I am helpful."));
    }
}
