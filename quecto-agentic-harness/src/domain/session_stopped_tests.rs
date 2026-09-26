use super::*;
use crate::domain::message::ToolCall;

fn call(id: &str, name: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: "{}".into(),
    }
}

#[test]
fn an_unanswered_call_gets_an_error_result_after_its_answered_siblings() {
    let mut messages = vec![
        Message::user("go"),
        Message::assistant("", vec![call("a", "bash"), call("b", "write")]),
        Message::tool("a", "done"),
    ];
    assert_eq!(
        answer_unfinished_tool_calls(&mut messages, "the run was stopped"),
        1
    );
    assert_eq!(messages.len(), 4);
    let added = &messages[3];
    assert_eq!(added.role, Role::Tool);
    assert_eq!(added.tool_call_id.as_deref(), Some("b"));
    assert_eq!(added.tool_name.as_deref(), Some("write"));
    assert!(added.is_error);
    assert_eq!(
        added.content,
        "the run was stopped before this call finished; whether it had any effect is unknown"
    );
}

#[test]
fn a_result_is_placed_before_the_next_message_not_at_the_end() {
    let mut messages = vec![
        Message::assistant("", vec![call("a", "bash")]),
        Message::user("later"),
        Message::assistant("", vec![call("b", "read")]),
        Message::tool("b", "text"),
    ];
    assert_eq!(answer_unfinished_tool_calls(&mut messages, "stopped"), 1);
    let order: Vec<(Role, Option<&str>)> = messages
        .iter()
        .map(|m| (m.role.clone(), m.tool_call_id.as_deref()))
        .collect();
    assert_eq!(
        order,
        vec![
            (Role::Assistant, None),
            (Role::Tool, Some("a")),
            (Role::User, None),
            (Role::Assistant, None),
            (Role::Tool, Some("b")),
        ]
    );
}

#[test]
fn a_complete_transcript_is_left_alone() {
    let mut messages = vec![
        Message::user("go"),
        Message::assistant("", vec![call("a", "bash")]),
        Message::tool("a", "done"),
        Message::assistant("finished", vec![]),
    ];
    let before: Vec<uuid::Uuid> = messages.iter().map(Message::id).collect();
    assert_eq!(answer_unfinished_tool_calls(&mut messages, "stopped"), 0);
    let after: Vec<uuid::Uuid> = messages.iter().map(Message::id).collect();
    assert_eq!(before, after);
}

#[test]
fn every_call_of_a_message_with_no_results_is_answered_in_call_order() {
    let mut messages = vec![Message::assistant(
        "",
        vec![call("a", "bash"), call("b", "grep")],
    )];
    assert_eq!(answer_unfinished_tool_calls(&mut messages, "stopped"), 2);
    let ids: Vec<Option<&str>> = messages[1..]
        .iter()
        .map(|m| m.tool_call_id.as_deref())
        .collect();
    assert_eq!(ids, vec![Some("a"), Some("b")]);
}
