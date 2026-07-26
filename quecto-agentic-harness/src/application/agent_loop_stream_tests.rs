use super::*;
use crate::domain::message::{LlmResponse, StopReason, ThinkingBlock, ToolCall};

fn response(
    content: Option<&str>,
    tool_calls: Vec<ToolCall>,
    thinking_blocks: Vec<ThinkingBlock>,
    stop_reason: Option<StopReason>,
) -> LlmResponse {
    LlmResponse {
        content: content.map(str::to_string),
        tool_calls,
        usage: None,
        stop_reason,
        thinking_blocks,
    }
}

#[test]
fn empty_stream_detection_requires_no_text_tools_or_thinking() {
    assert!(is_empty_streamed_response(&response(
        None,
        vec![],
        vec![],
        None
    )));
    assert!(is_empty_streamed_response(&response(
        Some(""),
        vec![],
        vec![],
        None
    )));
    assert!(!is_empty_streamed_response(&response(
        Some("text"),
        vec![],
        vec![],
        None
    )));
    assert!(!is_empty_streamed_response(&response(
        None,
        vec![ToolCall {
            id: "tc".into(),
            name: "bash".into(),
            arguments: "{}".into(),
        }],
        vec![],
        None,
    )));
    assert!(!is_empty_streamed_response(&response(
        None,
        vec![],
        vec![ThinkingBlock::Normal {
            thinking: "reasoning".into(),
            signature: "sig".into(),
        }],
        None,
    )));
}

#[test]
fn empty_stream_error_message_distinguishes_max_tokens_from_provider_empty() {
    assert_eq!(
        empty_stream_error_message(&response(None, vec![], vec![], Some(StopReason::MaxTokens))),
        "stream completed without assistant output: stop_reason=max_tokens"
    );
    assert_eq!(
        empty_stream_error_message(&response(None, vec![], vec![], Some(StopReason::EndTurn))),
        "HTTP 503: stream completed without assistant output"
    );
    assert_eq!(
        empty_stream_error_message(&response(None, vec![], vec![], None)),
        "HTTP 503: stream completed without assistant output"
    );
}
