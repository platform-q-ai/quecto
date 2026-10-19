use super::*;
use crate::domain::conversation::value_objects::message::{
    LlmResponse, StopReason, ThinkingBlock, ToolCall,
};

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

/// #2162 review: encrypted reasoning alone is no output — a reply with only
/// that is still an empty stream (retried), never a blank final answer.
#[test]
fn encrypted_reasoning_alone_is_an_empty_stream() {
    let only_encrypted = vec![
        crate::domain::conversation::value_objects::message::ThinkingBlock::EncryptedReasoning {
            origin: "o".into(),
            leads_to: None,
            item: "{}".into(),
        },
    ];
    assert!(is_empty_streamed_response(&response(
        None,
        vec![],
        only_encrypted,
        None
    )));
}
