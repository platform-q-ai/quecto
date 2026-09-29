//! STUB (#2272 red phase).
use std::sync::Arc;

use super::OverRepository;
use crate::application::swarm::dto::{SubmitTaskRequest, TaskTransition};
use crate::application::swarm::ports::{BoardEncoding, BoardRepository, Clock};
use crate::domain::swarm::{BoardError, RefusalKind};

pub struct SubmitTask {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    encoding: Arc<dyn BoardEncoding>,
}

impl SubmitTask {
    pub fn new(
        repository: Arc<dyn BoardRepository>,
        clock: Arc<dyn Clock + Send + Sync>,
        encoding: Arc<dyn BoardEncoding>,
    ) -> Self {
        Self {
            repository,
            clock,
            encoding,
        }
    }

    /// # Errors
    /// Pending #2272.
    pub fn execute(&self, _request: SubmitTaskRequest) -> Result<TaskTransition, BoardError> {
        let _ = (&self.repository, &self.clock, &self.encoding);
        Err(BoardError::new(RefusalKind::Internal, "pending #2272"))
    }
}

impl OverRepository for SubmitTask {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
            encoding: self.encoding.clone(),
        }
    }
}

#[cfg(test)]
#[path = "submit_task_tests.rs"]
mod tests;
