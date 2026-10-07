//! A swarm coordinator's parent wakes on the run's state, not on every idle
//! turn (#2467). The coordinator's harness reads its board at each idle
//! boundary and sends `swarm_state`; once a child has sent one, its turn-end
//! note waits for the next, which holds it (workers busy), names the state
//! (finished, idle with nothing in flight) or, when the board could not be
//! read, sends the ordinary turn-end note. A child that never sends one keeps
//! today's note on every turn end.

use super::super::subagent_registry::{NotificationTx, SubagentRegistry};

/// How long a held coordinator may go without a new turn before its parent
/// hears it has been quiet.
pub(crate) const QUIET_AFTER: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// Whether `agent_id`'s turn end waits for its next `swarm_state` (it has
/// sent one before), marking it deferred. Not wired yet.
pub(super) fn defer_completion(registry: &SubagentRegistry, agent_id: &str) -> bool {
    let _ = (registry, agent_id);
    false
}

/// Classify a `swarm_state` event. Not wired yet.
pub(super) fn classify_swarm_state(
    registry: &SubagentRegistry,
    notify_tx: Option<&NotificationTx>,
    agent_id: &str,
    sequence: u64,
    value: &serde_json::Value,
    quiet_after: std::time::Duration,
) {
    let _ = (registry, notify_tx, agent_id, sequence, value, quiet_after);
}
