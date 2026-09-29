//! STUB (#2272 red phase).
use std::sync::Arc;

use super::OverRepository;
use crate::application::swarm::dto::{BlockTaskRequest, TaskTransition};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{BoardError, RefusalKind};

pub struct BlockTask {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl BlockTask {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// Pending #2272.
    pub fn execute(&self, _request: BlockTaskRequest) -> Result<TaskTransition, BoardError> {
        let _ = (&self.repository, &self.clock);
        Err(BoardError::new(RefusalKind::Internal, "pending #2272"))
    }
}

impl OverRepository for BlockTask {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "block_task_tests.rs"]
mod tests;
