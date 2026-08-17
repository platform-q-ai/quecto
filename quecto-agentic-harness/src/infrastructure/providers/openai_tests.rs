use super::*;
use crate::domain::message::Message;
use crate::domain::provider::ChatRequest;
use crate::domain::tool::ToolDefinition;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn test_openai_oauth_jwt(account_id: &str) -> String {
    use base64::Engine;
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!(
        r#"{{"https://api.openai.com/auth":{{"chatgpt_account_id":"{}"}}}}"#,
        account_id
    ));
    format!("{}.{}.sig", header, payload)
}

#[test]
fn test_openai_provider_name() {
    let provider = OpenAiProvider::new("sk-test".to_string(), None);
    assert_eq!(provider.name(), "openai");
}

#[test]
fn test_custom_openai_compatible_provider_disables_oauth_headers() {
    let provider = OpenAiProvider::with_client_and_name_and_oauth_headers(
        "spark",
        test_openai_oauth_jwt("acct_test"),
        None,
        reqwest::Client::new(),
        false,
    );
    assert_eq!(provider.name(), "spark");
    assert!(provider.account_id.is_none());
}

#[test]
fn test_openai_provider_custom_base() {
    let provider = OpenAiProvider::new(
        "sk-test".to_string(),
        Some("http://localhost:8080".to_string()),
    );
    assert_eq!(provider.api_base, "http://localhost:8080");
}

#[test]
fn parse_response_preserves_non_stream_reasoning_fields() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "final",
                "reasoning": "visible thought",
                "reasoning_content": "ignored fallback"
            }
        }]
    });
    let resp = OpenAiProvider::parse_response(&body).unwrap();
    assert_eq!(resp.content.as_deref(), Some("final"));
    match &resp.thinking_blocks[0] {
        crate::domain::message::ThinkingBlock::Normal {
            thinking,
            signature,
        } => {
            assert_eq!(thinking, "visible thought");
            assert!(signature.is_empty());
        }
        other => panic!("unexpected thinking block: {other:?}"),
    }
}

#[test]
fn parse_response_drops_oversized_non_stream_reasoning_fields() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "final",
                "reasoning": "r".repeat(crate::infrastructure::providers::openai::openai_sse_parser::MAX_OPENAI_SSE_REASONING_BYTES + 1)
            }
        }]
    });
    let resp = OpenAiProvider::parse_response(&body).unwrap();
    assert!(resp.thinking_blocks.is_empty());
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
    // Non-streaming chat reports `context_tokens: None` (gauge falls back to
    // `prompt_tokens`) even though `total_tokens` is present — only the SSE
    // paths surface `total_tokens`. Locks the #996/PR-999 behaviour parity.
    assert_eq!(usage.context_tokens, None);
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
        parameters_schema: r#"{"type":"object","properties":{"command":{"type":"string"}}}"#.into(),
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
    let result = openai_sse_parser::parse_sse_response(sse).unwrap();
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
    let result = openai_sse_parser::parse_sse_response(sse).unwrap();
    assert!(result.content.is_none());
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].id, "call_1");
    assert_eq!(result.tool_calls[0].name, "bash");
    assert!(result.tool_calls[0].arguments.contains("ls"));
}

#[test]
fn test_parse_sse_empty() {
    let sse = "data: [DONE]\n";
    let result = openai_sse_parser::parse_sse_response(sse).unwrap();
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
