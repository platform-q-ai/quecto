//! #2349 review M1: the ladder removes whole call/result exchanges. An
//! assistant message with tool calls and every one of its results go or
//! stay together, so no request ever carries a call without its result or
//! a result without its call (OpenAI chat completions rejects either).

use super::*;
use crate::domain::conversation::value_objects::message::{Message, Role, ToolCall};
use std::collections::BTreeSet;

/// Every call has its result and every result its call.
fn assert_paired(messages: &[Message], context: &str) {
    let calls: BTreeSet<&str> = messages
        .iter()
        .filter(|m| m.role == Role::Assistant)
        .flat_map(|m| m.tool_calls.iter().map(|tc| tc.id.as_str()))
        .collect();
    let results: BTreeSet<&str> = messages
        .iter()
        .filter(|m| m.role == Role::Tool)
        .filter_map(|m| m.tool_call_id.as_deref())
        .collect();
    assert_eq!(
        calls, results,
        "{context}: calls and results must pair exactly"
    );
}

fn call(id: &str, name: &str) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments: "{}".to_string(),
    }
}

fn caller(turn: u32, calls: Vec<ToolCall>) -> Message {
    let mut msg = Message::assistant("", calls);
    msg.turn = Some(turn);
    msg
}

/// A tool result of `turn`; spilled (recallable, so stubbable) or not.
fn result(turn: u32, id: &str, content: String, spilled: bool) -> Message {
    let mut msg = Message::tool(id.to_string(), content);
    msg.tool_name = Some("bash".to_string());
    msg.turn = Some(turn);
    msg.spill_id = spilled.then(|| format!("turn{turn}:{id}"));
    msg
}

fn summary(turn: u32, id: &str) -> Message {
    let mut msg = result(
        turn,
        id,
        format!("{{\"members\":[]}} {}", "s ".repeat(300)),
        true,
    );
    msg.tool_name = Some("swarm".to_string());
    msg
}

fn done(turn: u32) -> Message {
    let mut msg = Message::assistant("done", vec![]);
    msg.turn = Some(turn);
    msg
}

fn big(words: usize) -> String {
    "plain ".repeat(words)
}

/// The review's case: a parallel `[bash call-b, swarm call-1]` response
/// whose summary is in the pinned recent turn. The drop rung must keep
/// `call-b`'s result with it rather than orphan the call.
#[test]
fn a_pinned_results_parallel_sibling_result_is_never_orphaned() {
    let mut messages = vec![
        Message::system("system"),
        Message::user("go"),
        caller(1, vec![call("call-c", "bash")]),
        result(1, "call-c", big(800), false),
        caller(2, vec![call("call-b", "bash"), call("call-1", "swarm")]),
        result(2, "call-b", big(1_500), false),
        summary(2, "call-1"),
        done(2),
    ];

    enforce_context_ceiling_ladder(&mut messages, 50, 1);

    assert_paired(&messages, "after the ladder");
    assert!(
        messages
            .iter()
            .any(|m| m.tool_call_id.as_deref() == Some("call-1")),
        "the pinned turn's summary stays"
    );
}

/// Dropping stops once under budget: it must never stop between an
/// assistant message and its results (here, after dropping the call and
/// the large result `a`, the rest fits, but `b` would be orphaned).
#[test]
fn the_drop_rung_never_stops_inside_an_exchange() {
    let mut messages = vec![
        Message::system("s"),
        Message::user("go"),
        caller(1, vec![call("a", "bash"), call("b", "bash")]),
        result(1, "a", big(600), false),
        result(1, "b", big(20), false),
        done(2),
    ];
    let mut last = done(3);
    last.content = big(300);
    messages.push(last);

    enforce_context_ceiling_ladder(&mut messages, 1_000, 0);

    assert_paired(&messages, "after the drop rung");
}

/// A mixed conversation: parallel calls, spilled and unspilled results,
/// conversation text.
fn mixed_conversation() -> Vec<Message> {
    let mut messages = vec![Message::system("system prompt"), Message::user("run it")];
    for turn in 1..=12u32 {
        let spilled = turn % 3 != 0;
        let mut calls = vec![call(&format!("t{turn}-a"), "bash")];
        if turn % 2 == 0 {
            calls.push(call(&format!("t{turn}-s"), "swarm"));
        }
        if turn % 4 == 1 {
            calls.push(call(&format!("t{turn}-c"), "grep"));
        }
        let ids: Vec<(String, String)> = calls
            .iter()
            .map(|c| (c.id.clone(), c.name.clone()))
            .collect();
        let mut assistant = caller(turn, calls);
        assistant.content = format!("thinking about turn {turn} {}", big(turn as usize * 7));
        assistant.spill_id = (turn % 2 == 1).then(|| format!("turn{turn}:msg:assistant"));
        messages.push(assistant);
        for (id, name) in ids {
            messages.push(match name.as_str() {
                "swarm" => summary(turn, &id),
                _ => result(turn, &id, big(40 * turn as usize), spilled),
            });
        }
    }
    messages.push(done(13));
    messages
}

/// Both rungs, every budget and pin: calls and results always pair.
#[test]
fn every_rung_keeps_calls_and_results_paired_at_any_budget() {
    let total = crate::application::context_pruning::estimate_total_tokens(&mixed_conversation());
    for pin in 0..=3u32 {
        for budget in (0..=total + 100).step_by(97) {
            let context = format!("pin {pin}, budget {budget}");
            let mut messages = mixed_conversation();
            assert_paired(&messages, &format!("{context}: the input"));
            enforce_context_ceiling_ladder(&mut messages, budget, pin);
            assert_paired(&messages, &format!("{context}: the ladder"));
        }
    }
}
