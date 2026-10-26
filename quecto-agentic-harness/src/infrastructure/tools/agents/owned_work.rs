//! Whether a trusted root parent's own sub-agents still run (#2708). The
//! registry holds exactly the agents this harness launched (and what they
//! relayed of theirs), so any starting or running entry is work it owns;
//! retained idle or dead rows are not.
use super::subagent_registry::SubagentRegistry;
use crate::domain::agents::value_objects::turn_outcome::WorkLiveness;

/// The registry's read: `Live` while any entry starts or runs, `Empty`
/// when none does (or the harness has no spawn tool to own one), and
/// `Unknown` when the registry cannot be read: never assumed empty.
pub fn registry_liveness(registry: Option<&SubagentRegistry>) -> WorkLiveness {
    let Some(registry) = registry else {
        return WorkLiveness::Empty;
    };
    match registry.lock() {
        Ok(entries) if entries.values().any(|entry| entry.status.is_active()) => WorkLiveness::Live,
        Ok(_) => WorkLiveness::Empty,
        Err(_) => WorkLiveness::Unknown,
    }
}

#[cfg(test)]
#[path = "owned_work_tests.rs"]
mod issue_2708_liveness_tests;
