// Anthropic adapter: impl LlmProvider for AnthropicProvider.

use std::future::Future;
use std::pin::Pin;

use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Role, ToolCall, UsageInfo};
use crate::domain::provider::{ChatRequest, LlmProvider};

/// Anthropic LLM provider.
#[derive(Debug)]
pub struct AnthropicProvider {
    api_key: String,
    api_base: String,
    client: reqwest::Client,
}

impl AnthropicProvider {
    pub fn new(api_key: String, api_base: Option<String>) -> Self {
        Self {
            api_key,
            api_base: api_base.unwrap_or_else(|| "https://api.anthropic.com".to_string()),
            client: reqwest::Client::new(),
        }
    }

    /// Build the JSON request body for Anthropic Messages API.
    fn build_request_body(request: &ChatRequest<'_>) -> (Option<String>, serde_json::Value) {
        let messages = request.messages;
        let tools = request.tools;
        let model = request.model;
        let max_tokens = request.max_tokens;
        let temperature = request.temperature;
        let mut system_prompt: Option<String> = None;
        let mut api_messages: Vec<serde_json::Value> = Vec::new();

        for m in messages {
            match m.role {
                Role::System => {
                    system_prompt = Some(m.content.clone());
                }
                Role::User => {
                    api_messages.push(serde_json::json!({
                        "role": "user",
                        "content": m.content,
                    }));
                }
                Role::Assistant => {
                    if !m.tool_calls.is_empty() {
                        let mut content_blocks: Vec<serde_json::Value> = Vec::new();
                        if !m.content.is_empty() {
                            content_blocks.push(serde_json::json!({
                                "type": "text",
                                "text": m.content,
                            }));
                        }
                        for tc in &m.tool_calls {
                            let input: serde_json::Value =
                                serde_json::from_str(&tc.arguments).unwrap_or_default();
                            content_blocks.push(serde_json::json!({
                                "type": "tool_use",
                                "id": tc.id,
                                "name": tc.name,
                                "input": input,
                            }));
                        }
                        api_messages.push(serde_json::json!({
                            "role": "assistant",
                            "content": content_blocks,
                        }));
                    } else {
                        api_messages.push(serde_json::json!({
                            "role": "assistant",
                            "content": m.content,
                        }));
                    }
                }
                Role::Tool => {
                    api_messages.push(serde_json::json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": m.tool_call_id.as_deref().unwrap_or(""),
                            "content": m.content,
                        }],
                    }));
                }
            }
        }

        let mut body = serde_json::json!({
            "model": model,
            "messages": api_messages,
            "max_tokens": max_tokens,
            "temperature": temperature,
        });

        if let Some(ref sys) = system_prompt {
            body["system"] = serde_json::Value::String(sys.clone());
        }

        if !tools.is_empty() {
            let tool_defs: Vec<serde_json::Value> = tools
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
                .collect();
            body["tools"] = serde_json::Value::Array(tool_defs);
        }

        (system_prompt, body)
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
            let response = self
                .client
                .post(&url)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", "2023-06-01")
                .header("Content-Type", "application/json")
                .json(&body)
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
        let messages = vec![Message {
            role: Role::User,
            content: "Hi".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        }];
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
        let messages = vec![Message {
            role: Role::User,
            content: "list files".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        }];
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
        let messages = vec![Message {
            role: Role::User,
            content: "Hi".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        }];
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
        let messages = vec![
            Message {
                role: Role::System,
                content: "You are helpful.".to_string(),
                tool_calls: vec![],
                tool_call_id: None,
            },
            Message {
                role: Role::User,
                content: "Hi".to_string(),
                tool_calls: vec![],
                tool_call_id: None,
            },
        ];
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
