// Persist/recovery coverage for Anthropic non-stream thinking blocks (#1231).
// Split from anthropic_thinking_tests.rs to stay within the 750-line limit.

use super::*;

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
    let result = AnthropicProvider::parse_response(&body, false, &[]).unwrap();
    assert_eq!(result.content.as_deref(), Some("The answer is 42"));
    assert_eq!(result.thinking_blocks.len(), 1);
    match &result.thinking_blocks[0] {
        crate::domain::message::ThinkingBlock::Normal {
            thinking,
            signature,
        } => {
            assert_eq!(thinking, "Let me reason through this...");
            assert!(signature.is_empty());
        }
        other => panic!("expected normal thinking block, got {other:?}"),
    }
}

#[test]
fn test_parse_response_keeps_redacted_thinking_placeholder() {
    let body = serde_json::json!({
        "content": [
            {"type": "thinking", "thinking": "visible", "signature": "sig"},
            {"type": "redacted_thinking", "data": "secret-redacted-blob"},
            {"type": "text", "text": "answer"}
        ]
    });
    let result = AnthropicProvider::parse_response(&body, false, &[]).unwrap();
    assert_eq!(result.content.as_deref(), Some("answer"));
    assert_eq!(result.thinking_blocks.len(), 2);
    match &result.thinking_blocks[1] {
        crate::domain::message::ThinkingBlock::Redacted { data } => {
            assert_eq!(data, "secret-redacted-blob");
        }
        other => panic!("expected redacted thinking block, got {other:?}"),
    }
}
