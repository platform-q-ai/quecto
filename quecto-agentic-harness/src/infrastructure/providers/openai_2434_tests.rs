//! #2434 review round 1: what a chat-completions request carries after a
//! turn that ended with no reply, and that it never carries an assistant
//! entry with no text and no call.
use super::OpenAiProvider;
use crate::application::providers::ports::ChatRequest;
use crate::domain::message::{Message, ThinkingBlock, ToolCall};

fn body(messages: &[Message]) -> serde_json::Value {
    let request = ChatRequest {
        trace: None,
        admission: None,
        messages,
        tools: &[],
        model: "accounts/fireworks/models/glm-5p3",
        max_tokens: 256,
        temperature: 0.2,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    OpenAiProvider::build_chat_completions_body_for_test("fireworks", &request)
}

fn roles(body: &serde_json::Value) -> Vec<String> {
    body["messages"]
        .as_array()
        .expect("messages")
        .iter()
        .filter_map(|m| m["role"].as_str().map(str::to_string))
        .collect()
}

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

#[test]
fn a_prompt_after_tool_results_follows_them() {
    let messages = vec![
        Message::user("start the job"),
        call(),
        Message::tool("call_1", "started"),
        Message::user("is it done?"),
    ];
    let body = body(&messages);
    assert_eq!(
        roles(&body),
        ["user", "assistant", "tool", "user"],
        "{body}"
    );
    assert_eq!(body["messages"][3]["content"], "is it done?");
}

/// An assistant entry with no text and no call (an empty final answer, or
/// `reasoning_content` alone, which this request never replays) is left out.
#[test]
fn an_assistant_entry_with_nothing_to_send_is_left_out() {
    for text in ["", " \n"] {
        let mut reply = Message::assistant(text, vec![]);
        reply.thinking_blocks = vec![ThinkingBlock::Normal {
            thinking: "reasoning_content".into(),
            signature: String::new(),
        }];
        let messages = vec![Message::user("q"), reply, Message::user("again")];
        let body = body(&messages);
        assert_eq!(roles(&body), ["user", "user"], "{text:?}: {body}");
    }
}
