//! Whether a trusted root parent's own sub-agents still run (#2708). The
//! registry holds exactly the agents this harness launched (and what they
//! relayed of theirs), so any starting or running entry is work it owns;
//! retained idle or dead rows are not.
use super::subagent_registry::SubagentRegistry;
use crate::domain::agents::value_objects::turn_outcome::WorkLiveness;

/// The registry's read: `Live` while any entry starts or runs, or an idle
/// coordinator it launched still holds its run's work (its last report a
/// hold), `Empty`
/// when none does (or the harness has no spawn tool to own one), and
/// `Unknown` when the registry cannot be read: never assumed empty.
pub fn registry_liveness(registry: Option<&SubagentRegistry>) -> WorkLiveness {
    match registry.map(|registry| registry.lock()) {
        Some(Ok(entries)) if entries.values().any(live) => WorkLiveness::Live,
        None | Some(Ok(_)) => WorkLiveness::Empty,
        Some(Err(_)) => WorkLiveness::Unknown,
    }
}

fn live(entry: &super::subagent_registry::SubagentEntry) -> bool {
    use super::subagent_registry::SubagentStatus::{Idle, Running, Starting};
    let holds = entry.coordinator_wake.holds_work();
    matches!(
        (&entry.status, holds),
        (Starting | Running, _) | (Idle, true)
    )
}

#[cfg(test)]
#[path = "owned_work_tests.rs"]
mod issue_2708_liveness_tests;
