//! Application-level environment control use case (#1369 slice 2).
//!
//! `get_containers`, `kill_container`, and ref/name resolution live here.
//! Interface handlers (tool/UDS) may only decode arguments, delegate to this
//! use case, and encode its results. The kill side effect (terminating member
//! agents and running the environment's retained kill argv) stays behind
//! [`EnvironmentKillPort`] in infrastructure.

use std::sync::Arc;

use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentTarget,
};
use crate::domain::subagent_launch::LaunchFuture;

/// Infrastructure port that tears one environment down: terminate every
/// member agent, then run the environment's retained kill argv exactly once.
pub trait EnvironmentKillPort: Send + Sync {
    fn kill_environment<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> LaunchFuture<'a, Result<(), String>>;
}

/// Use case over the session's authoritative environment registry.
pub struct EnvironmentControlUseCase {
    registry: EnvironmentRegistry,
    kill_port: Arc<dyn EnvironmentKillPort>,
}

impl std::fmt::Debug for EnvironmentControlUseCase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvironmentControlUseCase")
            .field("registry", &self.registry)
            .finish_non_exhaustive()
    }
}

impl EnvironmentControlUseCase {
    pub fn new(registry: EnvironmentRegistry, kill_port: Arc<dyn EnvironmentKillPort>) -> Self {
        Self {
            registry,
            kill_port,
        }
    }

    /// List every environment this session has committed — running, empty,
    /// stopped, and failed — straight from the authoritative registry.
    pub fn get_containers(&self) -> Vec<EnvironmentRecord> {
        self.registry.entries()
    }

    /// Kill one environment by ref or name: claim exclusively (no
    /// double-kill), run the retained kill exactly once, and commit stopped
    /// only after success. Failure persists a retryable cleanup-failed state.
    pub async fn kill_container(
        &self,
        target: &EnvironmentTarget,
    ) -> Result<EnvironmentRecord, String> {
        let resolved = self.registry.resolve(target).map_err(|e| e.to_string())?;
        // Refuse before claiming: a script set with no `kill` must leave the
        // environment Running and its members untouched, so joins keep working
        // and final-member exit still runs the retained cleanup fallback.
        if resolved.retained_kill_argv.is_empty() {
            return Err(format!(
                "environment {} has no retained kill argv; its script set does not support kill_container",
                resolved.environment_ref
            ));
        }
        let claim = self
            .registry
            .begin_kill(&resolved.environment_ref)
            .map_err(|e| e.to_string())?;
        // Re-read under the claim so the kill sees the members as of the
        // moment the claim was granted.
        let record = self
            .registry
            .get(&resolved.environment_ref)
            .unwrap_or(resolved);
        match self.kill_port.kill_environment(&record).await {
            Ok(()) => {
                self.registry.complete_kill(claim);
                Ok(record)
            }
            Err(e) => {
                self.registry.fail_kill(claim, &e);
                Err(format!(
                    "environment {} cleanup failed: {e}; state is cleanup-failed, retry kill_container",
                    record.environment_ref
                ))
            }
        }
    }
}
