// #2342: superseded snapshots. A tool result its tool names a whole
// snapshot of some state (`Message::snapshot_key`, a full swarm `summary`)
// is superseded by any newer result with the same key: the older ones
// collapse to their recall stubs, and the newest is never demoted by any
// dial, so the latest state always stays in full. Pure policy over the
// message list: no retention handle, no store.
//
// A swarm member rereads the board's summary every few turns, each a
// complete snapshot of it; before this, every earlier one stayed in the
// conversation in full (the owner's runs: 6-19% of a member's input).

use super::collapse_message;
use crate::domain::message::{Message, Role};

/// Whether `msg` is a live (not collapsed) tool result with a snapshot key.
fn live_snapshot(msg: &Message) -> Option<&'static str> {
    match msg.role {
        Role::Tool if !msg.is_collapsed => msg.snapshot_key,
        _ => None,
    }
}

/// The index of the newest live result of each snapshot key.
fn newest_indices(messages: &[Message]) -> Vec<(&'static str, usize)> {
    let mut newest: Vec<(&'static str, usize)> = Vec::new();
    for (index, msg) in messages.iter().enumerate() {
        let Some(key) = live_snapshot(msg) else {
            continue;
        };
        match newest.iter_mut().find(|(seen, _)| *seen == key) {
            Some(entry) => entry.1 = index,
            None => newest.push((key, index)),
        }
    }
    newest
}

/// Collapse every spilled tool result a newer result with the same
/// snapshot key supersedes, to its recall stub (the content was spilled at
/// creation, so it stays recallable). A result never spilled keeps its
/// content: its stub could not be recalled. Only results the model has
/// seen are superseded (#2213): two summaries of one in-flight batch both
/// reach the model once. Returns how many it collapsed; a pass that finds
/// nothing new changes nothing, so it rewrites no prompt prefix.
pub fn collapse_superseded_snapshots(messages: &mut [Message]) -> usize {
    let newest = newest_indices(messages);
    let seen_end = super::messages::ceiling::in_flight_start(messages);
    let mut collapsed = 0;
    for (index, msg) in messages[..seen_end].iter_mut().enumerate() {
        let Some(key) = live_snapshot(msg) else {
            continue;
        };
        let superseded = newest
            .iter()
            .any(|&(newest_key, newest_index)| newest_key == key && newest_index > index);
        if superseded && msg.spill_id.is_some() {
            collapse_message(msg);
            debug_assert!(msg.is_collapsed, "a superseded snapshot is now a stub");
            collapsed += 1;
        }
    }
    collapsed
}

/// Per message: whether it is the newest live result of its snapshot key,
/// or the assistant message that called it, which no pruning dial may
/// demote (a result whose call is gone is dropped from the request).
pub fn newest_snapshots(messages: &[Message]) -> Vec<bool> {
    let mut flags = vec![false; messages.len()];
    for (_, index) in newest_indices(messages) {
        flags[index] = true;
        let call_id = messages[index].tool_call_id.as_deref();
        let caller = messages[..index].iter().rposition(|m| {
            m.role == Role::Assistant
                && m.tool_calls
                    .iter()
                    .any(|tc| Some(tc.id.as_str()) == call_id)
        });
        if let Some(caller) = caller {
            flags[caller] = true;
        }
    }
    flags
}

#[cfg(test)]
#[path = "context_pruning_snapshots_tests.rs"]
mod tests;
