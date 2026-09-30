// #2342: superseded snapshots. A tool result its tool names a whole
// snapshot of some state (`Message::snapshot_key`, a full swarm `summary`)
// is superseded by any newer result with the same key: the older ones
// collapse to their recall stubs, and the newest is never demoted by any
// dial, so the latest state always stays in full. Pure policy over the
// message list: no retention handle, no store.

use crate::domain::message::Message;

/// Collapse every spilled tool result a newer result with the same
/// snapshot key supersedes. Returns how many it collapsed.
pub fn collapse_superseded_snapshots(_messages: &mut [Message]) -> usize {
    0
}

/// Per message: whether it is the newest live result of its snapshot key,
/// or the assistant message that called it, which no pruning dial may
/// demote (a result whose call is gone is dropped from the request).
pub fn newest_snapshots(messages: &[Message]) -> Vec<bool> {
    vec![false; messages.len()]
}

#[cfg(test)]
#[path = "context_pruning_snapshots_tests.rs"]
mod tests;
