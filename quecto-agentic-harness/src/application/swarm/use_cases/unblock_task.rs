//! STUB (#2272 red phase).
use std::sync::Arc;

use super::OverRepository;
use crate::application::swarm::dto::{TaskTransition, UnblockTaskRequest};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{BoardError, RefusalKind};

pub struct UnblockTask {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl UnblockTask {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// Pending #2272.
    pub fn execute(&self, _request: UnblockTaskRequest) -> Result<TaskTransition, BoardError> {
        let _ = (&self.repository, &self.clock);
        Err(BoardError::new(RefusalKind::Internal, "pending #2272"))
    }
}

impl OverRepository for UnblockTask {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "unblock_task_tests.rs"]
mod tests;
