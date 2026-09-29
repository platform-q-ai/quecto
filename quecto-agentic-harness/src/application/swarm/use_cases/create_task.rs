//! Stub (#2272 red phase).
use std::sync::Arc;

use crate::application::swarm::dto::CreateTaskRequest;
use crate::application::swarm::ports::{BoardEncoding, BoardRepository, Clock};
use crate::domain::swarm::BoardError;

pub struct CreateTask {
    repository: Arc<dyn BoardRepository>,
}

impl CreateTask {
    pub fn new(
        repository: Arc<dyn BoardRepository>,
        clock: Arc<dyn Clock + Send + Sync>,
        encoding: Arc<dyn BoardEncoding>,
    ) -> Self {
        let _ = (clock, encoding);
        Self { repository }
    }

    /// # Errors
    /// Always, until implemented.
    pub fn execute(
        &self,
        request: CreateTaskRequest,
    ) -> Result<crate::application::swarm::dto::CreatedTask, BoardError> {
        let _ = (&self.repository, request);
        Err(BoardError::new("pending #2272"))
    }
}

#[cfg(test)]
#[path = "create_task_tests.rs"]
mod tests;
