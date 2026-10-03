//! #2434 review round 1: the Responses API `input` after a turn that ended
//! with no reply, and that it never carries an assistant item with no text.
use super::*;
use crate::domain::message::{ThinkingBlock, ToolCall};

fn call() -> Message {
    Message::assistant(
        "",
        vec![ToolCall {
            id: "call_1".into(),
            name: "background".into(),
            arguments: "{}".into(),
        }],
    )
}

fn kinds(input: &[serde_json::Value]) -> Vec<String> {
    input
        .iter()
        .map(|item| {
            item["type"]
                .as_str()
                .or_else(|| item["role"].as_str())
                .unwrap_or_default()
                .to_string()
        })
        .collect()
}

#[test]
fn a_prompt_after_tool_results_follows_them() {
    let messages = vec![
        Message::user("start the job"),
        call(),
        Message::tool("call_1", "started"),
        Message::user("is it done?"),
    ];
    let (_, input) = CodexProvider::build_input(&messages);
    assert_eq!(
        kinds(&input),
        ["user", "function_call", "function_call_output", "user"],
        "{input:?}"
    );
    assert_eq!(input[3]["content"], "is it done?");
}

/// An assistant message with no text (an empty final answer, or reasoning
/// alone) sends no item: neither an empty answer nor reasoning that leads
/// to nothing.
#[test]
fn an_assistant_message_with_no_text_sends_nothing() {
    for text in ["", " \n"] {
        let mut reply = Message::assistant(text, vec![]);
        reply.thinking_blocks = vec![
            ThinkingBlock::Normal {
                thinking: "a summary".into(),
                signature: String::new(),
            },
            ThinkingBlock::EncryptedReasoning {
                origin: "chatgpt:model".into(),
                leads_to: None,
                item: r#"{"type":"reasoning","encrypted_content":"opaque"}"#.into(),
            },
        ];
        let messages = vec![Message::user("q"), reply, Message::user("again")];
        let (_, input) = CodexProvider::build_input_for(&messages, "chatgpt:model");
        assert_eq!(kinds(&input), ["user", "user"], "{text:?}: {input:?}");
    }
}
