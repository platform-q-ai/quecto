use super::*;
use crate::domain::conversation::value_objects::message::ToolCall;

fn call(id: &str, name: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: "{}".into(),
    }
}

fn start(messages: &[Message]) -> uuid::Uuid {
    messages[0].id()
}

#[test]
fn an_unanswered_call_gets_an_error_result_after_its_answered_siblings() {
    let mut messages = vec![
        Message::user("go"),
        Message::assistant("", vec![call("a", "bash"), call("b", "write")]),
        Message::tool("a", "done"),
    ];
    let run_start = start(&messages);
    assert_eq!(
        answer_unfinished_tool_calls(&mut messages, run_start, "the run was stopped"),
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
    let run_start = start(&messages);
    assert_eq!(
        answer_unfinished_tool_calls(&mut messages, run_start, "stopped"),
        1
    );
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
    let run_start = start(&messages);
    assert_eq!(
        answer_unfinished_tool_calls(&mut messages, run_start, "stopped"),
        0
    );
    let after: Vec<uuid::Uuid> = messages.iter().map(Message::id).collect();
    assert_eq!(before, after);
}

#[test]
fn every_call_of_a_message_with_no_results_is_answered_in_call_order() {
    let mut messages = vec![Message::assistant(
        "",
        vec![call("a", "bash"), call("b", "grep")],
    )];
    let run_start = start(&messages);
    assert_eq!(
        answer_unfinished_tool_calls(&mut messages, run_start, "stopped"),
        2
    );
    let ids: Vec<Option<&str>> = messages[1..]
        .iter()
        .map(|m| m.tool_call_id.as_deref())
        .collect();
    assert_eq!(ids, vec![Some("a"), Some("b")]);
}

/// Review finding: a call left unanswered by an earlier run (a crash) is
/// not relabelled as stopped by this one.
#[test]
fn calls_before_the_run_start_are_left_alone() {
    let prompt = Message::user("this run");
    let run_start = prompt.id();
    let mut messages = vec![
        Message::assistant("", vec![call("old", "bash")]),
        prompt,
        Message::assistant("", vec![call("new", "bash")]),
    ];
    assert_eq!(
        answer_unfinished_tool_calls(&mut messages, run_start, "stopped"),
        1
    );
    let answered: Vec<Option<&str>> = messages
        .iter()
        .filter(|m| m.role == Role::Tool)
        .map(|m| m.tool_call_id.as_deref())
        .collect();
    assert_eq!(answered, vec![Some("new")]);
}

/// Round 2 finding: a run start that is gone (pruned) falls back to the
/// messages not yet saved, never to history an earlier run saved.
#[test]
fn a_pruned_run_start_answers_only_unsaved_messages() {
    let mut saved = Message::assistant("", vec![call("old", "bash")]);
    saved.ordinal = Some(7);
    let mut messages = vec![saved, Message::assistant("", vec![call("new", "bash")])];
    let gone = Message::user("pruned").id();
    assert_eq!(
        answer_unfinished_tool_calls(&mut messages, gone, "stopped"),
        1
    );
    assert_eq!(messages[2].tool_call_id.as_deref(), Some("new"));
    // Everything saved already: nothing is this run's to answer.
    let mut saved_only = vec![messages.remove(0)];
    assert_eq!(
        answer_unfinished_tool_calls(&mut saved_only, gone, "stopped"),
        0
    );
}

/// Review finding: pairing is by id, as the orphan filter pairs them: a
/// result that is not directly after its call still answers it.
#[test]
fn a_result_anywhere_answers_its_call() {
    let mut messages = vec![
        Message::assistant("", vec![call("a", "bash")]),
        Message::user("between"),
        Message::tool("a", "late"),
    ];
    let run_start = start(&messages);
    assert_eq!(
        answer_unfinished_tool_calls(&mut messages, run_start, "stopped"),
        0
    );
    assert_eq!(messages.len(), 3);
}
