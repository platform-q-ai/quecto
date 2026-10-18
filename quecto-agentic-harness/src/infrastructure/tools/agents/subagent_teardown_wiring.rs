//! The launch-side lifecycle use cases of one registry (#1936): what the
//! reaper, the monitor and the launch rollback invoke when a direct child
//! ends. This module only declares the handles a launcher holds and the
//! slot they are installed into; composition builds the use cases
//! (`composition::subagent_lifecycle`) over the same registry, event stream
//! and supervisor the `SpawnTool` holds, so every launcher of this process —
//! production, integration launchers, BDD fixtures — observes exits through
//! one path. The operator-facing kill tool is composed separately
//! (`composition::subagent_termination`) over the same adapters.
use std::sync::Arc;

use crate::application::subagents::use_cases::{CompensateFailedLaunch, ObserveOwnedChildExit};

/// The use cases a launcher hands its reaper, monitor and rollback.
#[derive(Clone)]
pub struct SubagentLifecycleUseCases {
    pub observe_exit: Arc<ObserveOwnedChildExit>,
    pub compensate_launch: Arc<CompensateFailedLaunch>,
}

/// Where a launcher reads its lifecycle use cases from: built empty with
/// the agent-control tools, filled once by composition alongside the
/// termination owners (the same registry and channels). A second install
/// is ignored: one lifecycle per harness. Cloning shares the slot.
#[derive(Clone, Default)]
pub struct SubagentLifecycleSlot(Arc<std::sync::OnceLock<SubagentLifecycleUseCases>>);

impl SubagentLifecycleSlot {
    /// Install the use cases; `true` when this call filled the slot.
    pub fn install(&self, use_cases: SubagentLifecycleUseCases) -> bool {
        self.0.set(use_cases).is_ok()
    }

    pub fn get(&self) -> Option<SubagentLifecycleUseCases> {
        self.0.get().cloned()
    }
}

impl std::fmt::Debug for SubagentLifecycleSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SubagentLifecycleSlot")
            .field("installed", &self.0.get().is_some())
            .finish()
    }
}
