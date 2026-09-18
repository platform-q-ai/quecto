//! The launch lifecycle a `SpawnTool` hands its reaper, monitor and
//! rollback (#1936): held through a slot composition fills, never built
//! here. The tool's channels are exposed so composition can build the use
//! cases over exactly the registry and channels the tool forwards on.
use super::spawn::SpawnTool;
use super::subagent_registry::NotificationTx;
use super::subagent_teardown_wiring::{SubagentLifecycleSlot, SubagentLifecycleUseCases};
use crate::domain::error::DomainError;

/// What a launch reports when no lifecycle was composed for its tool.
pub const NO_LIFECYCLE_COMPOSED: &str = "no subagent lifecycle is composed in this session";

impl SpawnTool {
    /// Read the lifecycle use cases from a slot shared with whoever fills
    /// it later (composition, once the tool is already registered).
    pub fn with_lifecycle_slot(mut self, slot: SubagentLifecycleSlot) -> Self {
        self.lifecycle = slot;
        self
    }

    /// Install composed lifecycle use cases directly: launchers and
    /// fixtures that compose their own over this tool's registry.
    pub fn with_lifecycle_use_cases(self, use_cases: SubagentLifecycleUseCases) -> Self {
        let installed = self.lifecycle.install(use_cases);
        debug_assert!(
            installed,
            "the lifecycle use cases are composed once per launcher"
        );
        self
    }

    /// Whether composition installed this tool's lifecycle use cases.
    pub fn lifecycle_composed(&self) -> bool {
        self.lifecycle.get().is_some()
    }

    /// The lifecycle use cases (#1936) the reaper, monitor and rollback of
    /// every child this tool launches report to. `execute` refuses a real
    /// launch while the slot is empty; a launch port driven directly (the
    /// contract suite) meets the same refusal here.
    pub(super) fn lifecycle_use_cases(&self) -> Result<SubagentLifecycleUseCases, DomainError> {
        self.lifecycle
            .get()
            .ok_or_else(|| DomainError::Tool(NO_LIFECYCLE_COMPOSED.to_string()))
    }

    /// The event stream this tool forwards child events on, if any.
    pub fn broadcast_tx(&self) -> Option<&tokio::sync::broadcast::Sender<String>> {
        self.broadcast_tx.as_ref()
    }

    /// The passive-note channel this tool posts to, if any.
    pub fn notify_tx(&self) -> Option<&NotificationTx> {
        self.notify_tx.as_ref()
    }

    /// The base directory this tool launches children with (the quecto
    /// home, where the overlay trust record lives), never a checkout.
    pub fn base_dir(&self) -> &std::path::Path {
        &self.base_dir
    }
}

#[cfg(test)]
#[path = "spawn_lifecycle_tests.rs"]
mod tests;
