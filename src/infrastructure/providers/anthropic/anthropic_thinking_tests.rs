// Extended thinking tests for Anthropic provider (#175).
// Split from anthropic_tests.rs to stay within the 750-line limit.

use super::*;
use crate::domain::message::Message;
use crate::domain::provider::ChatRequest;

#[test]
fn test_build_request_body_with_thinking_adds_thinking_param() {
    let messages = vec![Message::user("Think hard")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-sonnet-4-20250514",
        max_tokens: 16000,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: Some(crate::domain::provider::ThinkingLevel::Medium),
        cancel_flag: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body(&req);
    assert_eq!(body["thinking"]["type"], "enabled");
    assert_eq!(body["thinking"]["budget_tokens"], 10000);
    // Temperature must be excluded when thinking is enabled
    assert!(
        body.get("temperature").is_none(),
        "temperature must be excluded when thinking is enabled, got: {}",
        body
    );
}

#[test]
fn test_build_request_body_without_thinking_includes_temperature() {
    let messages = vec![Message::user("Hi")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-sonnet-4-20250514",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body(&req);
    assert!(body.get("thinking").is_none());
    assert!(
        body.get("temperature").is_some(),
        "temperature should be present when thinking is disabled"
    );
}

#[test]
fn test_build_request_body_thinking_bumps_max_tokens() {
    let messages = vec![Message::user("Think")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-3-5-sonnet-20241022",
        max_tokens: 4096,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: Some(crate::domain::provider::ThinkingLevel::High),
        cancel_flag: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body(&req);
    // max_tokens must be at least budget_tokens (16384) when thinking is enabled
    assert!(
        body["max_tokens"].as_u64().unwrap() >= 16384,
        "max_tokens should be at least budget_tokens, got: {}",
        body["max_tokens"]
    );
}

#[test]
fn test_parse_sse_response_with_thinking_blocks() {
    let raw = "\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":10}}}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"thinking\",\"thinking\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"Let me think about this...\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\"}\n\
\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Here is my answer\"}}\n\
\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\"}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":20}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n";

    let result = AnthropicProvider::parse_sse_response(raw).unwrap();
    // Thinking content should NOT appear in the text content
    assert_eq!(result.content.as_deref(), Some("Here is my answer"));
    // Thinking content is not included in tool_calls
    assert!(result.tool_calls.is_empty());
}

#[test]
fn test_parse_response_with_thinking_blocks() {
    let body = serde_json::json!({
        "content": [
            {"type": "thinking", "thinking": "Let me reason through this..."},
            {"type": "text", "text": "The answer is 42"}
        ],
        "usage": {"input_tokens": 10, "output_tokens": 5},
        "stop_reason": "end_turn"
    });
    let result = AnthropicProvider::parse_response(&body).unwrap();
    assert_eq!(result.content.as_deref(), Some("The answer is 42"));
}

#[test]
fn test_thinking_budget_tokens_levels() {
    use crate::domain::provider::ThinkingLevel;
    assert_eq!(ThinkingLevel::Low.budget_tokens(), 1024);
    assert_eq!(ThinkingLevel::Medium.budget_tokens(), 10_000);
    assert_eq!(ThinkingLevel::High.budget_tokens(), 16_384);
    assert_eq!(ThinkingLevel::Max.budget_tokens(), 32_768);
}
