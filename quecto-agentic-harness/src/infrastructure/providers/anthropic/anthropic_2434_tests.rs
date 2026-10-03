//! #2434 review round 1: what the Messages API is sent after a turn that
//! ended with no reply, and that it is never sent empty text.
use super::*;
use crate::domain::message::{ThinkingBlock, ToolCall};

fn call(text: &str) -> Message {
    Message::assistant(
        text,
        vec![ToolCall {
            id: "call_1".into(),
            name: "background".into(),
            arguments: "{}".into(),
        }],
    )
}

fn roles(api: &[serde_json::Value]) -> Vec<&str> {
    api.iter().filter_map(|m| m["role"].as_str()).collect()
}

/// A turn that ended empty left the tool results last: the next prompt
/// follows them as its own user turn.
#[test]
fn a_prompt_after_tool_results_follows_them() {
    let messages = vec![
        Message::user("start the job"),
        call(""),
        Message::tool("call_1", "started"),
        Message::user("is it done?"),
    ];
    let (_, api) = AnthropicProvider::build_messages(&messages, false);
    assert_eq!(
        roles(&api),
        ["user", "assistant", "user", "user"],
        "{api:?}"
    );
    assert_eq!(api[2]["content"][0]["type"], "tool_result");
    assert!(api[3].to_string().contains("is it done?"), "{api:?}");
}

/// An assistant message with no text and nothing else to send (an empty
/// final answer, or reasoning only another provider can read) is left out:
/// the API refuses empty content.
#[test]
fn an_assistant_message_with_nothing_to_send_is_left_out() {
    let unsigned = ThinkingBlock::Normal {
        thinking: "a Codex summary".into(),
        signature: String::new(),
    };
    for (text, thinking) in [
        ("", vec![]),
        ("  \n", vec![]),
        ("", vec![unsigned.clone()]),
        (" ", vec![unsigned]),
    ] {
        let mut reply = Message::assistant(text, vec![]);
        reply.thinking_blocks = thinking;
        let messages = vec![Message::user("q"), reply, Message::user("again")];
        let (_, api) = AnthropicProvider::build_messages(&messages, false);
        assert_eq!(roles(&api), ["user", "user"], "{text:?}: {api:?}");
    }
}

/// A reasoning-only reply keeps its signed thinking, which Anthropic reads
/// back, and sends no text part.
#[test]
fn a_reasoning_only_reply_keeps_its_signed_thinking_without_text() {
    let mut reply = Message::assistant(" \n", vec![]);
    reply.thinking_blocks = vec![ThinkingBlock::Normal {
        thinking: "thought".into(),
        signature: "sig".into(),
    }];
    let messages = vec![Message::user("q"), reply, Message::user("again")];
    let (_, api) = AnthropicProvider::build_messages(&messages, false);
    assert_eq!(
        api[1]["content"],
        serde_json::json!([{"type": "thinking", "thinking": "thought", "signature": "sig"}])
    );
}

#[test]
fn whitespace_beside_a_tool_call_is_not_sent_as_text() {
    let messages = vec![
        Message::user("start the job"),
        call(" \n"),
        Message::tool("call_1", "started"),
    ];
    let (_, api) = AnthropicProvider::build_messages(&messages, false);
    let blocks = api[1]["content"].as_array().expect("blocks");
    assert_eq!(blocks.len(), 1, "{blocks:?}");
    assert_eq!(blocks[0]["type"], "tool_use");
}
