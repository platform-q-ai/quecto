//! #2348: a large tool result the model has seen for a few turns collapses
//! to its recall stub, whatever the count dial says.

use super::*;
use crate::domain::message::{Message, Role, ToolCall};

const DIAL: LargeResultCollapse = LargeResultCollapse {
    over_tokens: 2_000,
    after_turns: 3,
};

/// About 4k estimated tokens of ASCII prose.
fn large() -> String {
    "the quick brown fox jumps over the lazy dog ".repeat(360)
}

/// About 100 estimated tokens.
fn small() -> String {
    "a short result line ".repeat(20)
}

/// A spilled `bash` result of `turn`.
fn result(turn: u32, content: String) -> Message {
    let mut msg = Message::tool(format!("call-{turn}"), content);
    msg.tool_name = Some("bash".to_string());
    msg.input_preview = Some(r#"{"command":"git log"}"#.to_string());
    msg.turn = Some(turn);
    msg.spill_id = Some(format!("turn{turn}:bash:0"));
    msg
}

/// The assistant message that called `result(turn, ..)`.
fn call(turn: u32) -> Message {
    let mut msg = Message::assistant(
        "",
        vec![ToolCall {
            id: format!("call-{turn}"),
            name: "bash".to_string(),
            arguments: r#"{"command":"git log"}"#.to_string(),
        }],
    );
    msg.turn = Some(turn);
    msg
}

/// One call and result per entry, in order: each later call is a model
/// response that saw every earlier result.
fn conversation(results: Vec<Message>) -> Vec<Message> {
    let mut messages = vec![Message::system("system"), Message::user("go")];
    for msg in results {
        messages.push(call(msg.turn.unwrap()));
        messages.push(msg);
    }
    messages
}

/// `seen` model responses (plain text) after the conversation so far.
fn responses(messages: &mut Vec<Message>, seen: u32) {
    for n in 0..seen {
        messages.push(Message::assistant(format!("reply {n}"), vec![]));
        messages.push(Message::user(format!("next {n}")));
    }
}

fn tool_results(messages: &[Message]) -> Vec<&Message> {
    messages.iter().filter(|m| m.role == Role::Tool).collect()
}

#[test]
fn a_large_result_seen_for_its_turns_collapses_to_a_recall_stub() {
    let mut messages = conversation(vec![result(1, large())]);
    responses(&mut messages, 3);

    assert_eq!(collapse_large_results(&mut messages, DIAL), 1);

    let results = tool_results(&messages);
    assert!(results[0].is_collapsed);
    assert!(
        results[0].content.contains("recall(\"turn1:bash:0\")"),
        "a collapsed result keeps its recall stub: {}",
        results[0].content
    );
}

#[test]
fn a_large_result_seen_for_fewer_turns_stays_in_full() {
    let mut messages = conversation(vec![result(1, large())]);
    responses(&mut messages, 2);

    assert_eq!(collapse_large_results(&mut messages, DIAL), 0);

    assert_eq!(tool_results(&messages)[0].content, large());
}

#[test]
fn a_later_call_counts_as_a_turn_that_saw_the_result() {
    // Tool-calling turns: results 2, 3 and 4 follow result 1, so their
    // calls are three responses that saw it; result 4 is in flight.
    let mut messages = conversation(vec![
        result(1, large()),
        result(2, small()),
        result(3, small()),
        result(4, large()),
    ]);

    assert_eq!(collapse_large_results(&mut messages, DIAL), 1);

    let results = tool_results(&messages);
    assert!(results[0].is_collapsed, "seen by three later calls");
    assert!(!results[3].is_collapsed, "the in-flight result is unseen");
}

#[test]
fn a_small_result_stays_in_full_however_long_it_is_seen() {
    let mut messages = conversation(vec![result(1, small())]);
    responses(&mut messages, 40);

    assert_eq!(collapse_large_results(&mut messages, DIAL), 0);

    assert_eq!(tool_results(&messages)[0].content, small());
}

#[test]
fn an_unseen_result_is_never_stubbed_even_at_zero_turns() {
    // #2213: the model has not seen a result after the last assistant
    // message; a dial of 0 turns still waits for one response.
    let zero = LargeResultCollapse {
        after_turns: 0,
        ..DIAL
    };
    let mut messages = conversation(vec![result(1, large())]);

    assert_eq!(collapse_large_results(&mut messages, zero), 0);
    assert!(!tool_results(&messages)[0].is_collapsed);

    responses(&mut messages, 1);
    assert_eq!(collapse_large_results(&mut messages, zero), 1);
}

#[test]
fn an_unspilled_result_is_never_stubbed() {
    let mut unspilled = result(1, large());
    unspilled.spill_id = None;
    let mut messages = conversation(vec![unspilled]);
    responses(&mut messages, 10);

    assert_eq!(collapse_large_results(&mut messages, DIAL), 0);
    assert_eq!(tool_results(&messages)[0].content, large());
}

#[test]
fn the_newest_snapshot_stays_in_full_however_large() {
    let mut summary = result(1, large());
    summary.snapshot_key = Some("swarm.summary");
    let mut messages = conversation(vec![summary]);
    responses(&mut messages, 10);

    assert_eq!(collapse_large_results(&mut messages, DIAL), 0);
    assert_eq!(tool_results(&messages)[0].content, large());
}

#[test]
fn a_collapse_keeps_every_call_and_result_pair_whole() {
    let mut messages = conversation(vec![result(1, large()), result(2, large())]);
    responses(&mut messages, 3);
    let before: Vec<(Role, Option<String>, usize)> = messages
        .iter()
        .map(|m| (m.role.clone(), m.tool_call_id.clone(), m.tool_calls.len()))
        .collect();

    assert_eq!(collapse_large_results(&mut messages, DIAL), 2);

    let after: Vec<(Role, Option<String>, usize)> = messages
        .iter()
        .map(|m| (m.role.clone(), m.tool_call_id.clone(), m.tool_calls.len()))
        .collect();
    assert_eq!(before, after, "no message, call or result id is removed");
}

#[test]
fn a_second_pass_collapses_nothing_more() {
    let mut messages = conversation(vec![result(1, large())]);
    responses(&mut messages, 3);
    assert_eq!(collapse_large_results(&mut messages, DIAL), 1);
    let first = messages.clone();

    assert_eq!(collapse_large_results(&mut messages, DIAL), 0);
    let contents = |m: &[Message]| m.iter().map(|m| m.content.clone()).collect::<Vec<_>>();
    assert_eq!(contents(&first), contents(&messages), "no prefix rewrite");
}

#[test]
fn the_disabled_rule_collapses_nothing() {
    let mut messages = conversation(vec![result(1, large())]);
    responses(&mut messages, 50);

    assert_eq!(
        collapse_large_results(&mut messages, LargeResultCollapse::DISABLED),
        0
    );
}

#[test]
fn a_result_whose_stub_is_no_cheaper_stays_in_full() {
    // A dial set below a stub's size must not grow the conversation.
    let tiny = LargeResultCollapse {
        over_tokens: 1,
        after_turns: 1,
    };
    let mut messages = conversation(vec![result(1, "ok".to_string())]);
    responses(&mut messages, 3);

    assert_eq!(collapse_large_results(&mut messages, tiny), 0);
    assert_eq!(tool_results(&messages)[0].content, "ok");
}
