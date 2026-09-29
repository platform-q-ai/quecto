//! `Workbench._snapshot` (#2270): the run and its members, as the lifecycle
//! reads them.
use std::sync::Arc;

use super::OverRepository;
use crate::application::swarm::board_operation::operation;
use crate::application::swarm::dto::RunSnapshotView;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError};

/// Through the operation gate as a read (`active=False, read_only=True`):
/// any member, a dead one included, reads it, and a run whose deadline has
/// come is ended as `budget-exhausted` first.
pub struct ReadRunSnapshot {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ReadRunSnapshot {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation refusal (no run, an unknown member), or the store's.
    pub fn execute(&self, member: &str) -> Result<RunSnapshotView, BoardError> {
        let reading = Access {
            read_only: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            member,
            reading,
            |transaction, run| {
                let control_generation = transaction.control_generation()?;
                let members = transaction.members()?;
                Ok(RunSnapshotView {
                    status: run.status.as_ref().map(|status| status.as_str().to_owned()),
                    coordinator: run.coordinator.clone(),
                    outcome: run.outcome.clone(),
                    control_generation,
                    deadline: run.deadline,
                    members,
                })
            },
        )
    }
}

impl OverRepository for ReadRunSnapshot {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "read_run_snapshot_tests.rs"]
mod tests;
