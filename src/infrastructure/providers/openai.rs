// OpenAI adapter: impl LlmProvider for OpenAiProvider.

use std::future::Future;
use std::pin::Pin;

use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Role, ToolCall, UsageInfo};
use crate::domain::provider::{ChatRequest, LlmProvider};

/// OpenAI-compatible LLM provider.
#[derive(Debug)]
pub struct OpenAiProvider {
    api_key: String,
    api_base: String,
    client: reqwest::Client,
}

impl OpenAiProvider {
    pub fn new(api_key: String, api_base: Option<String>) -> Self {
        Self {
            api_key,
            api_base: api_base.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            client: reqwest::Client::new(),
        }
    }

    /// Build the JSON request body for OpenAI chat completions.
    fn build_request_body(request: &ChatRequest<'_>) -> serde_json::Value {
        let messages = request.messages;
        let tools = request.tools;
        let model = request.model;
        let max_tokens = request.max_tokens;
        let msgs: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                };
                let mut obj = serde_json::json!({
                    "role": role,
                    "content": m.content,
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

        let usage = body["usage"].as_object().map(|u| UsageInfo {
            prompt_tokens: u["prompt_tokens"].as_u64().unwrap_or(0) as u32,
            completion_tokens: u["completion_tokens"].as_u64().unwrap_or(0) as u32,
        });

        Ok(LlmResponse {
            content,
            tool_calls,
            usage,
        })
    }
}

impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn chat(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        let body = Self::build_request_body(&request);
        let url = format!("{}/chat/completions", self.api_base);

        Box::pin(async move {
            let response = self
                .client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
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
                    "HTTP {} from OpenAI: {}",
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
    fn test_openai_provider_name() {
        let provider = OpenAiProvider::new("sk-test".to_string(), None);
        assert_eq!(provider.name(), "openai");
    }

    #[test]
    fn test_openai_provider_custom_base() {
        let provider = OpenAiProvider::new(
            "sk-test".to_string(),
            Some("http://localhost:8080".to_string()),
        );
        assert_eq!(provider.api_base, "http://localhost:8080");
    }

    #[tokio::test]
    async fn test_chat_text_response() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "Hello!" },
                "finish_reason": "stop"
            }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
        });
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("sk-test".to_string(), Some(server.uri()));
        let messages = vec![Message {
            role: Role::User,
            content: "Hi".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        }];
        let req = ChatRequest {
            messages: &messages,
            tools: &[],
            model: "gpt-4",
            max_tokens: 1024,
            temperature: 0.7,
        };
        let result = provider.chat(req).await;
        assert!(result.is_ok(), "chat should succeed: {:?}", result);
        let response = result.unwrap();
        assert_eq!(response.content.as_deref(), Some("Hello!"));
        assert!(response.tool_calls.is_empty());
        assert!(response.usage.is_some());
        let usage = response.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 5);
    }

    #[tokio::test]
    async fn test_chat_with_tool_calls() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "id": "chatcmpl-456",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc",
                        "type": "function",
                        "function": {
                            "name": "exec",
                            "arguments": "{\"command\":\"ls\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": { "prompt_tokens": 20, "completion_tokens": 10, "total_tokens": 30 }
        });
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("sk-test".to_string(), Some(server.uri()));
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
            model: "gpt-4",
            max_tokens: 1024,
            temperature: 0.7,
        };
        let result = provider.chat(req).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].id, "call_abc");
        assert_eq!(response.tool_calls[0].name, "exec");
        assert!(response.tool_calls[0].arguments.contains("ls"));
    }

    #[tokio::test]
    async fn test_chat_server_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("sk-test".to_string(), Some(server.uri()));
        let messages = vec![Message {
            role: Role::User,
            content: "Hi".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        }];
        let req = ChatRequest {
            messages: &messages,
            tools: &[],
            model: "gpt-4",
            max_tokens: 1024,
            temperature: 0.7,
        };
        let result = provider.chat(req).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("500"), "error should mention status: {}", err);
    }

    #[tokio::test]
    async fn test_chat_includes_tools_in_request() {
        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "id": "chatcmpl-789",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "ok" },
                "finish_reason": "stop"
            }]
        });
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .expect(1)
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("sk-test".to_string(), Some(server.uri()));
        let messages = vec![Message {
            role: Role::User,
            content: "test".to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        }];
        let tools = vec![ToolDefinition {
            name: "read_file".to_string(),
            description: "Read a file".to_string(),
            parameters_schema: r#"{"type":"object"}"#.to_string(),
        }];
        let req = ChatRequest {
            messages: &messages,
            tools: &tools,
            model: "gpt-4",
            max_tokens: 1024,
            temperature: 0.7,
        };
        let result = provider.chat(req).await;
        assert!(result.is_ok());
    }
}
