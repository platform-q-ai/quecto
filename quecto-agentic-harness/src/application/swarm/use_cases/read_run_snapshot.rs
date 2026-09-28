//! `Workbench._snapshot` (#2270): red-phase skeleton.
use std::sync::Arc;

use crate::application::swarm::board_operation::{detail, end, operation, text};
use crate::application::swarm::dto::RunSnapshotView;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError};

pub struct ReadRunSnapshot {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ReadRunSnapshot {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// Not implemented yet.
    pub fn execute(&self, member: &str) -> Result<RunSnapshotView, BoardError> {
        let _ = detail([("member", text(member))]);
        operation(
            &*self.repository,
            &*self.clock,
            member,
            Access::default(),
            |transaction, _| {
                end(transaction, &*self.clock, member, "", "")?;
                Err(BoardError::new("not implemented yet (#2270)"))
            },
        )
    }
}

#[cfg(test)]
#[path = "read_run_snapshot_tests.rs"]
mod tests;
