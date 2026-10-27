//! Whether a trusted root parent's own sub-agents still run (#2708). The
//! registry holds exactly the agents this harness launched (and what they
//! relayed of theirs), so any starting or running entry is work it owns;
//! retained idle or dead rows are not.
use super::subagent_registry::SubagentRegistry;
use crate::domain::agents::value_objects::turn_outcome::WorkLiveness;

/// The registry's read, any `Live` entry winning: `Unknown` (never assumed
/// empty) when unreadable or a coordinator's work can't be told.
pub fn registry_liveness(registry: Option<&SubagentRegistry>) -> WorkLiveness {
    match registry.map(|registry| registry.lock()) {
        Some(Ok(entries)) => entries
            .values()
            .map(work)
            .fold(WorkLiveness::Empty, WorkLiveness::and),
        None => WorkLiveness::Empty,
        Some(Err(_)) => WorkLiveness::Unknown,
    }
}

fn work(entry: &super::subagent_registry::SubagentEntry) -> WorkLiveness {
    use super::subagent_registry::SubagentStatus::{Error, Idle, Running, Starting};
    match entry.status {
        Starting | Running => WorkLiveness::Live,
        Idle | Error => entry.coordinator_wake.work(),
        _ => WorkLiveness::Empty,
    }
}

#[cfg(test)]
#[path = "owned_work_tests.rs"]
mod issue_2708_liveness_tests;
