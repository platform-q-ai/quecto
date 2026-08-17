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

/// Regression (#996 item 9): the incremental streaming path reconstructs the
/// provider for its spawned task. It previously hardcoded the router name to
/// "anthropic", clobbering registry-built providers that carry a custom prefix
/// (e.g. "anthropic-oauth"). The derived `Clone` must preserve every field,
/// including `router_name`, so the task-local clone keeps the same identity.
#[test]
fn streaming_task_clone_preserves_custom_router_name() {
    let provider = AnthropicProvider::with_client_and_name(
        "sk-ant-test".to_string(),
        None,
        reqwest::Client::new(),
        "anthropic-oauth",
    );
    assert_eq!(provider.name(), "anthropic-oauth");
    let cloned = provider.clone();
    assert_eq!(
        cloned.name(),
        "anthropic-oauth",
        "streaming task clone must preserve the custom router name, not reset to 'anthropic'"
    );
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
        model: "claude-sonnet-4-6",
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
            "name": "bash",
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
        name: "bash".into(),
        description: "Execute a command".into(),
        parameters_schema: r#"{"type":"object","properties":{"command":{"type":"string"}}}"#.into(),
    }];
    let req = ChatRequest {
        messages: &messages,
        tools: &tools,
        model: "claude-sonnet-4-6",
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
    assert_eq!(response.tool_calls[0].id, "toolu_abc");
    assert_eq!(response.tool_calls[0].name, "bash");
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
        model: "claude-sonnet-4-6",
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
event: content_block_delta\n\
data: {\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n\
event: content_block_delta\n\
data: {\"delta\":{\"type\":\"text_delta\",\"text\":\" from Claude\"}}\n\n\
event: message_stop\n\
data: {}\n";
    let result = AnthropicProvider::parse_sse_response(sse, None).unwrap();
    assert_eq!(result.content.as_deref(), Some("Hello from Claude"));
    assert!(result.tool_calls.is_empty());
}

#[test]
fn test_parse_sse_tool_use() {
    let sse = "\
event: content_block_start\n\
data: {\"content_block\":{\"type\":\"tool_use\",\"id\":\"tu_1\",\"name\":\"bash\"}}\n\n\
event: content_block_delta\n\
data: {\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"cmd\\\"\"}}\n\n\
event: content_block_delta\n\
data: {\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\": \\\"ls\\\"}\"}}\n\n\
event: content_block_stop\n\
data: {}\n\n\
event: message_stop\n\
data: {}\n";
    let result = AnthropicProvider::parse_sse_response(sse, None).unwrap();
    assert!(result.content.is_none());
    assert_eq!(result.tool_calls.len(), 1);
    assert_eq!(result.tool_calls[0].id, "tu_1");
    assert_eq!(result.tool_calls[0].name, "bash");
    assert!(result.tool_calls[0].arguments.contains("ls"));
}

#[test]
fn test_parse_sse_empty_stops() {
    let sse = "event: message_stop\ndata: {}\n";
    let result = AnthropicProvider::parse_sse_response(sse, None).unwrap();
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
        model: "claude-sonnet-4-6",
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
        model: "claude-sonnet-4-6",
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
    assert_eq!(result.unwrap().content.as_deref(), Some("I am helpful."));
}

// --- #209: Shared reqwest::Client ---

#[test]
fn test_anthropic_provider_accepts_shared_client() {
    let client = reqwest::Client::new();
    let provider = AnthropicProvider::with_client("sk-ant-test".to_string(), None, client);
    assert_eq!(provider.name(), "anthropic");
}

// --- #178: is_error flag on tool result messages ---

#[test]
fn test_tool_result_message_includes_is_error_true() {
    let mut m = Message::tool("tc_1", "Error: file not found");
    m.is_error = true;
    let json = AnthropicProvider::build_tool_result_message(&m);
    let content = json["content"].as_array().expect("content should be array");
    assert_eq!(content[0]["is_error"], serde_json::Value::Bool(true));
}

#[test]
fn test_tool_result_message_includes_is_error_false() {
    let mut m = Message::tool("tc_1", "file contents here");
    m.is_error = false;
    let json = AnthropicProvider::build_tool_result_message(&m);
    let content = json["content"].as_array().expect("content should be array");
    assert_eq!(content[0]["is_error"], serde_json::Value::Bool(false));
}

// --- #177: Stop reason extraction ---

#[test]
fn test_parse_response_extracts_stop_reason_end_turn() {
    let body = serde_json::json!({
        "content": [{"type": "text", "text": "Hello"}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 10, "output_tokens": 5}
    });
    let response = AnthropicProvider::parse_response(&body, false, &[]).unwrap();
    assert_eq!(
        response.stop_reason,
        Some(crate::domain::message::StopReason::EndTurn)
    );
}

#[test]
fn test_parse_response_extracts_stop_reason_max_tokens() {
    let body = serde_json::json!({
        "content": [{"type": "text", "text": "Truncat"}],
        "stop_reason": "max_tokens",
        "usage": {"input_tokens": 10, "output_tokens": 100}
    });
    let response = AnthropicProvider::parse_response(&body, false, &[]).unwrap();
    assert_eq!(
        response.stop_reason,
        Some(crate::domain::message::StopReason::MaxTokens)
    );
}

#[test]
fn test_parse_response_extracts_stop_reason_tool_use() {
    let body = serde_json::json!({
        "content": [{"type": "tool_use", "id": "t1", "name": "bash", "input": {}}],
        "stop_reason": "tool_use",
        "usage": {"input_tokens": 10, "output_tokens": 5}
    });
    let response = AnthropicProvider::parse_response(&body, false, &[]).unwrap();
    assert_eq!(
        response.stop_reason,
        Some(crate::domain::message::StopReason::ToolUse)
    );
}

#[test]
fn test_parse_sse_extracts_stop_reason_from_message_delta() {
    let sse = "\
event: content_block_delta\n\
data: {\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n\
event: message_delta\n\
data: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":10}}\n\n\
event: message_stop\n\
data: {}\n";
    let result = AnthropicProvider::parse_sse_response(sse, None).unwrap();
    assert_eq!(
        result.stop_reason,
        Some(crate::domain::message::StopReason::EndTurn)
    );
}

// --- #180: Usage from SSE stream ---

#[test]
fn test_parse_sse_extracts_usage_from_message_start_and_delta() {
    let sse = "\
event: message_start\n\
data: {\"message\":{\"usage\":{\"input_tokens\":100,\"output_tokens\":0}}}\n\n\
event: content_block_delta\n\
data: {\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n\
event: message_delta\n\
data: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":50}}\n\n\
event: message_stop\n\
data: {}\n";
    let result = AnthropicProvider::parse_sse_response(sse, None).unwrap();
    let usage = result.usage.expect("should have usage");
    assert_eq!(usage.prompt_tokens, 100);
    assert_eq!(usage.completion_tokens, 50);
}

#[test]
fn test_parse_sse_extracts_cache_usage() {
    let sse = "\
event: message_start\n\
data: {\"message\":{\"usage\":{\"input_tokens\":100,\"output_tokens\":0,\"cache_read_input_tokens\":80,\"cache_creation_input_tokens\":20}}}\n\n\
event: content_block_delta\n\
data: {\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n\
event: message_delta\n\
data: {\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":50}}\n\n\
event: message_stop\n\
data: {}\n";
    let result = AnthropicProvider::parse_sse_response(sse, None).unwrap();
    let usage = result.usage.expect("should have usage");
    assert_eq!(usage.cache_read_tokens, Some(80));
    assert_eq!(usage.cache_write_tokens, Some(20));
    // #(bug): context occupancy must include cached tokens, not just the
    // non-cached `input_tokens` delta (100 + 80 + 20 = 200).
    assert_eq!(usage.context_tokens, Some(200));
}

#[test]
fn test_parse_response_extracts_cache_usage() {
    let body = serde_json::json!({
        "content": [{"type": "text", "text": "Hello"}],
        "stop_reason": "end_turn",
        "usage": {
            "input_tokens": 100,
            "output_tokens": 50,
            "cache_read_input_tokens": 80,
            "cache_creation_input_tokens": 20
        }
    });
    let response = AnthropicProvider::parse_response(&body, false, &[]).unwrap();
    let usage = response.usage.expect("should have usage");
    assert_eq!(usage.cache_read_tokens, Some(80));
    assert_eq!(usage.cache_write_tokens, Some(20));
    // Context occupancy = input_tokens + cache_read + cache_creation.
    assert_eq!(usage.context_tokens, Some(200));
    // Billing inputs stay separate: prompt_tokens must NOT absorb cache tokens,
    // otherwise cache reads would be double-charged in `ModelPricing::cost_for`.
    assert_eq!(usage.prompt_tokens, 100);
}

// --- #176: Prompt caching (cache_control markers) ---

#[test]
fn test_build_request_body_system_prompt_has_cache_control() {
    let messages = vec![Message::system("You are helpful."), Message::user("Hi")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-sonnet-4-6",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body_public(&req);
    // System prompt should be an array of content blocks with cache_control
    let system = &body["system"];
    assert!(
        system.is_array(),
        "system should be content block array, got: {}",
        system
    );
    let blocks = system.as_array().unwrap();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0]["type"], "text");
    assert_eq!(blocks[0]["text"], "You are helpful.");
    assert!(
        blocks[0]["cache_control"].is_object(),
        "should have cache_control"
    );
    assert_eq!(blocks[0]["cache_control"]["type"], "ephemeral");
}

#[test]
fn test_build_request_body_last_user_message_has_cache_control() {
    let messages = vec![
        Message::system("You are helpful."),
        Message::user("First message"),
        Message::user("Second message"),
    ];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-sonnet-4-6",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body_public(&req);
    let api_messages = body["messages"].as_array().unwrap();
    // Last user message should have cache_control on its content block
    let last_msg = api_messages.last().unwrap();
    let content = &last_msg["content"];
    // Content should be a block array with cache_control
    assert!(
        content.is_array(),
        "last user message content should be array for cache_control"
    );
    let blocks = content.as_array().unwrap();
    let last_block = blocks.last().unwrap();
    assert!(
        last_block["cache_control"].is_object(),
        "last block should have cache_control"
    );
    assert_eq!(last_block["cache_control"]["type"], "ephemeral");
}

// --- #187: Batch consecutive tool results ---

#[test]
fn test_build_messages_batches_consecutive_tool_results() {
    let messages = vec![
        Message::user("do stuff"),
        Message::assistant(
            "",
            vec![
                ToolCall {
                    id: "tc_1".into(),
                    name: "bash".into(),
                    arguments: "{}".into(),
                },
                ToolCall {
                    id: "tc_2".into(),
                    name: "read".into(),
                    arguments: "{}".into(),
                },
                ToolCall {
                    id: "tc_3".into(),
                    name: "ls".into(),
                    arguments: "{}".into(),
                },
            ],
        ),
        Message::tool("tc_1", "output 1"),
        Message::tool("tc_2", "output 2"),
        Message::tool("tc_3", "output 3"),
    ];
    let (_sys, api_messages) =
        AnthropicProvider::build_messages(&messages, "claude-opus-4-5", false);
    // user, assistant, then ONE user message with 3 tool_result blocks
    assert_eq!(
        api_messages.len(),
        3,
        "should have user + assistant + batched tool results"
    );
    let tool_msg = &api_messages[2];
    assert_eq!(tool_msg["role"], "user");
    let content = tool_msg["content"]
        .as_array()
        .expect("content should be array");
    assert_eq!(content.len(), 3, "should have 3 tool_result blocks");
    for block in content {
        assert_eq!(block["type"], "tool_result");
    }
}

#[test]
fn test_build_messages_single_tool_result_in_single_message() {
    let messages = vec![
        Message::user("do stuff"),
        Message::assistant(
            "",
            vec![ToolCall {
                id: "tc_1".into(),
                name: "bash".into(),
                arguments: "{}".into(),
            }],
        ),
        Message::tool("tc_1", "output"),
    ];
    let (_sys, api_messages) =
        AnthropicProvider::build_messages(&messages, "claude-opus-4-5", false);
    assert_eq!(api_messages.len(), 3);
    let tool_msg = &api_messages[2];
    let content = tool_msg["content"]
        .as_array()
        .expect("content should be array");
    assert_eq!(content.len(), 1, "should have 1 tool_result block");
    assert_eq!(content[0]["type"], "tool_result");
}

// --- #183: tool_choice parameter ---

#[test]
fn test_build_request_body_includes_tool_choice_auto() {
    let messages = vec![Message::user("Hi")];
    let tools = vec![ToolDefinition {
        name: "bash".into(),
        description: "Execute".into(),
        parameters_schema: "{}".into(),
    }];
    let req = ChatRequest {
        messages: &messages,
        tools: &tools,
        model: "claude-sonnet-4-6",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: Some(crate::domain::provider::ToolChoice::Auto),
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body_public(&req);
    assert_eq!(body["tool_choice"]["type"], "auto");
}

#[test]
fn test_build_request_body_includes_tool_choice_any() {
    let messages = vec![Message::user("Hi")];
    let tools = vec![ToolDefinition {
        name: "bash".into(),
        description: "Execute".into(),
        parameters_schema: "{}".into(),
    }];
    let req = ChatRequest {
        messages: &messages,
        tools: &tools,
        model: "claude-sonnet-4-6",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: Some(crate::domain::provider::ToolChoice::Any),
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body_public(&req);
    assert_eq!(body["tool_choice"]["type"], "any");
}

#[test]
fn test_build_request_body_includes_tool_choice_specific() {
    let messages = vec![Message::user("Hi")];
    let tools = vec![ToolDefinition {
        name: "bash".into(),
        description: "Execute".into(),
        parameters_schema: "{}".into(),
    }];
    let req = ChatRequest {
        messages: &messages,
        tools: &tools,
        model: "claude-sonnet-4-6",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: Some(crate::domain::provider::ToolChoice::Specific("bash".into())),
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body_public(&req);
    assert_eq!(body["tool_choice"]["type"], "tool");
    assert_eq!(body["tool_choice"]["name"], "bash");
}

// --- #186: metadata.user_id support ---

#[test]
fn test_build_request_body_includes_metadata_user_id() {
    let messages = vec![Message::user("Hi")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-sonnet-4-6",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: Some(crate::domain::provider::RequestMetadata {
            user_id: Some("telegram_12345".into()),
        }),
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body_public(&req);
    assert_eq!(body["metadata"]["user_id"], "telegram_12345");
}

#[test]
fn test_build_request_body_omits_metadata_when_none() {
    let messages = vec![Message::user("Hi")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-sonnet-4-6",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body_public(&req);
    assert!(body.get("metadata").is_none() || body["metadata"].is_null());
}

// --- #179: Beta headers for API key auth ---
// fine-grained-tool-streaming is now GA — the header must NOT be sent.
// The dedicated test for this lives in anthropic_thinking_tests.rs.

// --- normalize_messages clone-on-write tests (#374) ---

#[test]
fn test_normalize_messages_does_not_clone_unmodified_messages() {
    // Messages with no tool calls need no normalization — they should be
    // returned as borrowed Cow::Borrowed, not deep-cloned.
    let messages = vec![
        Message::user("hello"),
        Message::assistant("world", vec![]),
        Message::user("follow up"),
    ];

    let normalized = AnthropicProvider::normalize_messages(&messages);
    assert_eq!(normalized.len(), 3);

    // Verify pointer equality: each Cow::Borrowed should point to the
    // original message, not a clone.
    for (i, cow) in normalized.iter().enumerate() {
        let original_ptr = &messages[i] as *const Message;
        // Cow::Borrowed derefs to the original; Cow::Owned derefs to a new alloc.
        let normalized_ptr: *const Message = &**cow;
        assert_eq!(
            original_ptr, normalized_ptr,
            "message {} should be Cow::Borrowed (same pointer), not cloned",
            i
        );
    }
}

// Thinking tests are in anthropic_thinking_tests.rs (split for 750-line limit)
#[path = "anthropic_thinking_tests.rs"]
mod thinking_tests;

#[path = "anthropic_thinking_persist_tests.rs"]
mod thinking_persist_tests;

// Latest Opus model tests (split for 750-line limit)
#[path = "anthropic_latest_opus_tests.rs"]
mod latest_opus_tests;

// mod.rs region-coverage tests (split for 750-line limit)
#[path = "anthropic_mod_cov_tests.rs"]
mod mod_cov_tests;
