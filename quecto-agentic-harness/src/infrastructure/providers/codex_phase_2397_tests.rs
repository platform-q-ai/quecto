//! #2397: an assistant message's `phase` is fixed once it is sent, so the
//! input a conversation becomes only ever grows at its end and the prompt
//! cache keeps every earlier item.
use super::*;
use crate::domain::message::ToolCall;

fn call(id: &str) -> Message {
    let mut assistant = Message::assistant("", vec![]);
    assistant.tool_calls = vec![ToolCall {
        id: id.to_string(),
        name: "read".into(),
        arguments: r#"{"path":"a.rs"}"#.to_string(),
    }];
    assistant
}

/// Two turns of a conversation: a question, a tool call and its result, and
/// an answer each.
fn two_turns() -> Vec<Message> {
    vec![
        Message::system("Be concise."),
        Message::user("First question"),
        call("call_1"),
        Message::tool("call_1", "file one"),
        Message::assistant("First answer.", vec![]),
        Message::user("Second question"),
        call("call_2"),
        Message::tool("call_2", "file two"),
        Message::assistant("Second answer.", vec![]),
    ]
}

#[test]
fn a_new_turn_leaves_every_earlier_input_item_unchanged() {
    let before = two_turns();
    let mut after = before.clone();
    after.push(Message::user("Third question"));
    after.push(call("call_3"));
    after.push(Message::tool("call_3", "file three"));
    after.push(Message::assistant("Third answer.", vec![]));

    let (_, earlier) = CodexProvider::build_input(&before);
    let (_, later) = CodexProvider::build_input(&after);

    assert!(later.len() > earlier.len(), "the new turn adds items");
    for (index, item) in earlier.iter().enumerate() {
        assert_eq!(
            &later[index], item,
            "input item {index} changed when a turn was appended"
        );
    }
}

#[test]
fn every_answer_that_ended_its_turn_is_a_final_answer() {
    let (_, input) = CodexProvider::build_input(&two_turns());
    let answers: Vec<&serde_json::Value> = input
        .iter()
        .filter(|item| item["role"] == "assistant")
        .collect();
    assert_eq!(answers.len(), 2, "{input:?}");
    for answer in answers {
        assert_eq!(answer["phase"], "final_answer", "{answer}");
    }
}

#[test]
fn text_the_same_turn_continues_after_stays_commentary() {
    let messages = vec![
        Message::user("Go"),
        Message::assistant("Looking first.", vec![]),
        Message::assistant("Done.", vec![]),
    ];
    let (_, input) = CodexProvider::build_input(&messages);
    assert_eq!(input[1]["phase"], "commentary");
    assert_eq!(input[2]["phase"], "final_answer");
}
