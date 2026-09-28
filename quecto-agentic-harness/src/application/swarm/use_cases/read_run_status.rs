//! `Workbench._status` (#2270): red-phase skeleton.
use std::sync::Arc;

use crate::application::swarm::board_operation::atomic;
use crate::application::swarm::dto::RunStatusView;
use crate::application::swarm::ports::BoardRepository;
use crate::domain::swarm::BoardError;

pub struct ReadRunStatus {
    repository: Arc<dyn BoardRepository>,
}

impl ReadRunStatus {
    pub fn new(repository: Arc<dyn BoardRepository>) -> Self {
        Self { repository }
    }

    /// # Errors
    /// Not implemented yet.
    pub fn execute(&self) -> Result<RunStatusView, BoardError> {
        atomic(&*self.repository, false, |_| {
            Err(BoardError::new("not implemented yet (#2270)"))
        })
    }
}

#[cfg(test)]
#[path = "read_run_status_tests.rs"]
mod tests;
