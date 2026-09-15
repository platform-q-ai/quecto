//! Adapter of the `DelegatedChildrenRoster` port of a session transition
//! (D7 #1976, #1938): the sub-agent registry as the roster the transition
//! counts and replaces. A mapping only — what a live delegated row is —
//! the transaction decides what to do with the count.
use crate::application::sessions::ports::DelegatedChildrenRoster;
use crate::domain::session::SubagentLiveness;
use crate::infrastructure::tools::subagent_registry::{SubagentRegistry, SubagentStatus};

/// The registry's rows as the transition's roster.
pub struct RegistryDelegatedRoster(SubagentRegistry);

impl RegistryDelegatedRoster {
    pub fn new(registry: SubagentRegistry) -> Self {
        Self(registry)
    }
}

impl DelegatedChildrenRoster for RegistryDelegatedRoster {
    /// Rows this harness addresses as delegated agents that are still
    /// live and not exited.
    fn live_delegated_rows(&self) -> usize {
        self.0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter(|entry| {
                entry.delegated_identity().is_some()
                    && entry.persisted_liveness == SubagentLiveness::Live
                    && entry.status != SubagentStatus::Exited
            })
            .count()
    }

    fn clear_roster(&self) -> usize {
        let mut entries = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let dropped = entries.len();
        entries.clear();
        dropped
    }
}
