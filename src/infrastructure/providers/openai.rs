// OpenAI adapter: impl LlmProvider for OpenAiProvider.

use std::future::Future;
use std::pin::Pin;

use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Role, ToolCall, UsageInfo};
use crate::domain::provider::{ChatRequest, LlmProvider, StreamEvent};

/// OpenAI-compatible LLM provider.
#[derive(Debug)]
pub struct OpenAiProvider {
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
        let account_id = crate::infrastructure::auth::oauth::extract_openai_account_id(&api_key);
        Self {
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
            cache_read_tokens: None,
            cache_write_tokens: None,
            cost: None,
        });

        Ok(LlmResponse {
            content,
            tool_calls,
            usage,
            stop_reason: None,
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

        let response = request_builder
            .send()
            .await
            .map_err(|e| DomainError::Provider(format!("HTTP error: {}", e)))?;

        let status = response.status().as_u16();
        if status != 200 {
            let text = response.text().await.unwrap_or_default();
            return Err(DomainError::Provider(format!(
                "HTTP {} from OpenAI: {}",
                status, text
            )));
        }

        let full = response
            .text()
            .await
            .map_err(|e| DomainError::Provider(format!("failed to read stream: {}", e)))?;

        Self::parse_sse_response(&full)
    }

    /// Parse an SSE text stream into an assembled LlmResponse.
    fn parse_sse_response(raw: &str) -> Result<LlmResponse, DomainError> {
        let mut content = String::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        for line in raw.lines() {
            let line = line.trim();
            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };
            if data == "[DONE]" {
                break;
            }
            let chunk: serde_json::Value = serde_json::from_str(data).unwrap_or_default();
            let Some(choices) = chunk["choices"].as_array() else {
                continue;
            };
            for choice in choices {
                Self::apply_delta(&choice["delta"], &mut content, &mut tool_calls);
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
            stop_reason: None,
        })
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
                    .send(StreamEvent::Error(format!("HTTP error: {e}")))
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
                    "HTTP {status} from OpenAI: {text}"
                )))
                .await;
            return;
        }
        openai_sse::pump_sse_bytes(&mut response, &tx).await;
    }

    /// Maximum number of tool calls allowed in a single streaming response.
    const MAX_TOOL_CALLS: usize = 128;

    /// Apply a single SSE delta chunk to the accumulated content and tool calls.
    fn apply_delta(
        delta: &serde_json::Value,
        content: &mut String,
        tool_calls: &mut Vec<ToolCall>,
    ) {
        if let Some(text) = delta["content"].as_str() {
            content.push_str(text);
        }
        if let Some(tcs) = delta["tool_calls"].as_array() {
            for tc in tcs {
                let idx = tc["index"].as_u64().unwrap_or(0) as usize;
                if idx >= Self::MAX_TOOL_CALLS {
                    continue;
                }
                while tool_calls.len() <= idx {
                    tool_calls.push(ToolCall {
                        id: String::new(),
                        name: String::new(),
                        arguments: String::new(),
                    });
                }
                if let Some(id) = tc["id"].as_str() {
                    tool_calls[idx].id = id.to_string();
                }
                if let Some(name) = tc["function"]["name"].as_str() {
                    tool_calls[idx].name = name.to_string();
                }
                if let Some(args) = tc["function"]["arguments"].as_str() {
                    tool_calls[idx].arguments.push_str(args);
                }
            }
        }
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
            let request_builder = self
                .client
                .post(&url)
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

    fn chat_stream(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        let mut body = Self::build_request_body(&request);
        body["stream"] = serde_json::Value::Bool(true);
        let url = format!("{}/chat/completions", self.api_base);
        Box::pin(async move { self.stream_chat_with_body(body, &url).await })
    }

    fn chat_stream_incremental(
        &self,
        request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + '_>> {
        let mut body = Self::build_request_body(&request);
        body["stream"] = serde_json::Value::Bool(true);
        let url = format!("{}/chat/completions", self.api_base);
        let api_key = self.api_key.clone();
        let api_base = self.api_base.clone();
        let account_id = self.account_id.clone();
        let client = self.client.clone();
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::channel(64);
            tokio::spawn(async move {
                let provider = OpenAiProvider {
                    api_key,
                    api_base,
                    client,
                    account_id,
                };
                provider.pump_sse_incremental(body, &url, tx).await;
            });
            rx
        })
    }
}

#[path = "openai_sse.rs"]
mod openai_sse;

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
        let messages = vec![Message::user("Hi")];
        let req = ChatRequest {
            messages: &messages,
            tools: &[],
            model: "gpt-4",
            max_tokens: 1024,
            temperature: 0.7,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
            effort: None,
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
                            "name": "bash",
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
        let messages = vec![Message::user("list files")];
        let tools = vec![ToolDefinition {
            name: "bash".into(),
            description: "Execute a command".into(),
            parameters_schema: r#"{"type":"object","properties":{"command":{"type":"string"}}}"#
                .into(),
        }];
        let req = ChatRequest {
            messages: &messages,
            tools: &tools,
            model: "gpt-4",
            max_tokens: 1024,
            temperature: 0.7,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
            effort: None,
        };
        let result = provider.chat(req).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].id, "call_abc");
        assert_eq!(response.tool_calls[0].name, "bash");
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
        let messages = vec![Message::user("Hi")];
        let req = ChatRequest {
            messages: &messages,
            tools: &[],
            model: "gpt-4",
            max_tokens: 1024,
            temperature: 0.7,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
            effort: None,
        };
        let result = provider.chat(req).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("500"), "error should mention status: {}", err);
    }

    #[test]
    fn test_parse_sse_text_response() {
        let sse = "\
data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\
data: {\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\
data: [DONE]\n";
        let result = OpenAiProvider::parse_sse_response(sse).unwrap();
        assert_eq!(result.content.as_deref(), Some("Hello world"));
        assert!(result.tool_calls.is_empty());
    }

    #[test]
    fn test_parse_sse_tool_call() {
        let sse = "\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"bash\",\"arguments\":\"\"}}]}}]}\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"cmd\\\"\"}}]}}]}\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\": \\\"ls\\\"}\"}}]}}]}\n\
data: [DONE]\n";
        let result = OpenAiProvider::parse_sse_response(sse).unwrap();
        assert!(result.content.is_none());
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].id, "call_1");
        assert_eq!(result.tool_calls[0].name, "bash");
        assert!(result.tool_calls[0].arguments.contains("ls"));
    }

    #[test]
    fn test_parse_sse_empty() {
        let sse = "data: [DONE]\n";
        let result = OpenAiProvider::parse_sse_response(sse).unwrap();
        assert!(result.content.is_none());
        assert!(result.tool_calls.is_empty());
    }

    #[tokio::test]
    async fn test_chat_stream_with_mock() {
        let server = MockServer::start().await;
        let sse_body = "\
data: {\"choices\":[{\"delta\":{\"content\":\"Hi\"}}]}\n\n\
data: {\"choices\":[{\"delta\":{\"content\":\" there\"}}]}\n\n\
data: [DONE]\n\n";
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(sse_body)
                    .insert_header("content-type", "text/event-stream"),
            )
            .mount(&server)
            .await;

        let provider = OpenAiProvider::new("sk-test".to_string(), Some(server.uri()));
        let messages = vec![Message::user("Hi")];
        let req = ChatRequest {
            messages: &messages,
            tools: &[],
            model: "gpt-4",
            max_tokens: 1024,
            temperature: 0.7,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
            effort: None,
        };
        let result = provider.chat_stream(req).await;
        assert!(result.is_ok(), "stream should succeed: {:?}", result);
        let resp = result.unwrap();
        assert_eq!(resp.content.as_deref(), Some("Hi there"));
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
        let messages = vec![Message::user("test")];
        let tools = vec![ToolDefinition {
            name: "read".into(),
            description: "Read a file".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }];
        let req = ChatRequest {
            messages: &messages,
            tools: &tools,
            model: "gpt-4",
            max_tokens: 1024,
            temperature: 0.7,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
            effort: None,
        };
        let result = provider.chat(req).await;
        assert!(result.is_ok());
    }

    // --- #209: Shared reqwest::Client ---

    #[test]
    fn test_openai_provider_accepts_shared_client() {
        let client = reqwest::Client::new();
        let provider = OpenAiProvider::with_client("sk-test".to_string(), None, client);
        assert_eq!(provider.name(), "openai");
    }
}
