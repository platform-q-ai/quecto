//! #2342: a newer whole snapshot of a state supersedes the older ones; the
//! newest is never demoted by any dial.

use super::*;
use crate::application::context_pruning::collapse_tool_results_over_limit;
use crate::application::context_pruning::messages::enforce_context_ceiling_ladder;
use crate::domain::message::{Message, Role, ToolCall};

const SUMMARY: &str = "swarm.summary";

/// A spilled tool result of `turn`, a snapshot of `key` when one is given.
fn result(turn: u32, content: &str, key: Option<&'static str>) -> Message {
    let mut msg = Message::tool(format!("call-{turn}"), content.to_string());
    msg.tool_name = Some("swarm".to_string());
    msg.input_preview = Some(r#"{"op":"summary"}"#.to_string());
    msg.turn = Some(turn);
    msg.spill_id = Some(format!("turn{turn}:swarm:0"));
    msg.snapshot_key = key;
    msg
}

/// The assistant message that called `result(turn, ..)`.
fn call(turn: u32) -> Message {
    let mut msg = Message::assistant(
        "",
        vec![ToolCall {
            id: format!("call-{turn}"),
            name: "swarm".to_string(),
            arguments: r#"{"op":"summary"}"#.to_string(),
        }],
    );
    msg.turn = Some(turn);
    msg
}

/// A conversation of one call and result per entry, in order.
fn conversation(results: Vec<Message>) -> Vec<Message> {
    let mut messages = vec![Message::system("system"), Message::user("go")];
    for msg in results {
        messages.push(call(msg.turn.unwrap()));
        messages.push(msg);
    }
    messages
}

fn full_summary(n: u32) -> String {
    format!(
        r#"{{"members":[],"tasks":[{n}],"event_cursor":{n}}} {}"#,
        "x".repeat(400)
    )
}

fn tool_results(messages: &[Message]) -> Vec<&Message> {
    messages.iter().filter(|m| m.role == Role::Tool).collect()
}

#[test]
fn a_newer_snapshot_supersedes_every_older_one_of_its_key() {
    let mut messages = conversation(vec![
        result(1, &full_summary(1), Some(SUMMARY)),
        result(2, &full_summary(2), Some(SUMMARY)),
        result(3, &full_summary(3), Some(SUMMARY)),
    ]);

    assert_eq!(collapse_superseded_snapshots(&mut messages), 2);

    let results = tool_results(&messages);
    assert!(results[0].is_collapsed && results[1].is_collapsed);
    assert!(
        results[0].content.contains("recall(\"turn1:swarm:0\")"),
        "a superseded snapshot keeps its recall stub: {}",
        results[0].content
    );
    assert!(!results[2].is_collapsed, "the newest stays in full");
    assert_eq!(results[2].content, full_summary(3));
}

#[test]
fn a_second_pass_supersedes_nothing_more() {
    let mut messages = conversation(vec![
        result(1, &full_summary(1), Some(SUMMARY)),
        result(2, &full_summary(2), Some(SUMMARY)),
    ]);
    assert_eq!(collapse_superseded_snapshots(&mut messages), 1);
    let after = messages.clone();

    assert_eq!(collapse_superseded_snapshots(&mut messages), 0);
    assert!(
        after
            .iter()
            .zip(&messages)
            .all(|(a, b)| a.content == b.content),
        "an idempotent pass rewrites no prompt prefix"
    );
}

#[test]
fn results_that_are_no_snapshot_or_of_another_key_stay() {
    let mut messages = conversation(vec![
        result(1, "an inbox answer: the only copy of a message", None),
        result(2, &full_summary(2), Some(SUMMARY)),
        result(3, "the tasks of another state", Some("other.state")),
        result(4, "another inbox answer", None),
        result(5, &full_summary(5), Some(SUMMARY)),
    ]);

    assert_eq!(collapse_superseded_snapshots(&mut messages), 1);

    let results = tool_results(&messages);
    let collapsed: Vec<bool> = results.iter().map(|m| m.is_collapsed).collect();
    assert_eq!(collapsed, [false, true, false, false, false]);
}

#[test]
fn an_unspilled_snapshot_is_never_stubbed() {
    // Its recall() would not resolve: it stays in full (allowlist: only a
    // spilled result may collapse).
    let mut unspilled = result(1, &full_summary(1), Some(SUMMARY));
    unspilled.spill_id = None;
    let mut messages = conversation(vec![unspilled, result(2, &full_summary(2), Some(SUMMARY))]);

    assert_eq!(collapse_superseded_snapshots(&mut messages), 0);
    assert!(tool_results(&messages).iter().all(|m| !m.is_collapsed));
}

#[test]
fn only_the_newest_live_snapshot_of_each_key_and_its_call_are_marked() {
    let messages = conversation(vec![
        result(1, &full_summary(1), Some(SUMMARY)),
        result(2, "other", Some("other.state")),
        result(3, &full_summary(3), Some(SUMMARY)),
        result(4, "plain", None),
    ]);

    let newest = newest_snapshots(&messages);

    assert_eq!(newest.len(), messages.len());
    let marked: Vec<usize> = (0..messages.len()).filter(|&i| newest[i]).collect();
    // system, user, then (call, result) pairs: results at 3, 5, 7, 9. The
    // newest summary (7) and the other key's (5) are marked with the calls
    // that asked for them (6, 4): a result whose call went would be dropped
    // from the request as an orphan.
    assert_eq!(marked, [4, 5, 6, 7]);
}

#[test]
fn the_tool_dial_never_collapses_the_newest_snapshot() {
    let mut messages = conversation(vec![
        result(1, &full_summary(1), Some(SUMMARY)),
        result(2, "plain two", None),
        result(3, "plain three", None),
    ]);
    messages.push(Message::assistant("done", vec![]));

    // Without the snapshot, three live results over a dial of 1 collapse 2.
    let collapsed = collapse_tool_results_over_limit(&mut messages, 1);

    let results = tool_results(&messages);
    assert!(
        !results[0].is_collapsed,
        "the latest board state stays visible"
    );
    assert_eq!(collapsed, 1, "the snapshot is not counted either");
    assert!(results[1].is_collapsed && !results[2].is_collapsed);
}

#[test]
fn the_ceiling_ladder_never_stubs_the_newest_snapshot() {
    let mut messages = conversation(vec![
        result(1, &full_summary(1), Some(SUMMARY)),
        result(2, &"plain ".repeat(200), None),
        result(3, &"plain ".repeat(200), None),
        result(4, &"plain ".repeat(200), None),
    ]);
    messages.push(Message::assistant("done", vec![]));

    let outcome = enforce_context_ceiling_ladder(&mut messages, 50, 0);

    let snapshot = messages
        .iter()
        .find(|m| m.snapshot_key == Some(SUMMARY))
        .expect("the snapshot is never dropped");
    assert!(
        !snapshot.is_collapsed,
        "the latest board state stays in full"
    );
    assert_eq!(snapshot.content, full_summary(1));
    assert!(
        messages
            .iter()
            .any(|m| m.tool_calls.iter().any(|tc| tc.id == "call-1")),
        "its call stays too, or the request would drop it as an orphan"
    );
    assert!(outcome.collapsed_to_stubs > 0, "the others were stubbed");
}
