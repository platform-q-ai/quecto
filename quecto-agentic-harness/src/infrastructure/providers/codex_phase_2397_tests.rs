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
fn an_answer_at_the_end_keeps_its_phase_when_more_assistant_text_follows() {
    // Review round 1, L1: no lookahead, so even the last item of a sent
    // request is serialized the same way once anything is appended.
    let before = vec![Message::user("Go"), Message::assistant("Part one.", vec![])];
    let mut after = before.clone();
    after.push(Message::assistant("Part two.", vec![]));
    let (_, earlier) = CodexProvider::build_input(&before);
    let (_, later) = CodexProvider::build_input(&after);
    assert_eq!(earlier[1], later[1], "the earlier answer changed");
    assert_eq!(later[1]["phase"], "final_answer");
    assert_eq!(later[2]["phase"], "final_answer");
}

#[test]
fn text_sent_in_place_of_orphaned_tool_calls_is_commentary() {
    // Review round 1, L2: the orphan fallback carries text that came with
    // tool calls, so it never ended its turn.
    let mut assistant = Message::assistant("I was going to read a file.", vec![]);
    assistant.tool_calls = vec![ToolCall {
        id: "call_orphan".to_string(),
        name: "read".into(),
        arguments: "{}".to_string(),
    }];
    let (_, input) = CodexProvider::build_input(&[Message::user("Go"), assistant]);
    let text = input
        .iter()
        .find(|item| item["role"] == "assistant")
        .expect("the fallback text is sent");
    assert_eq!(text["phase"], "commentary", "{text}");
}
