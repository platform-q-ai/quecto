//! Stub (#2272 red phase).
use std::sync::Arc;

use crate::application::swarm::dto::ReadTaskRequest;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::BoardError;

pub struct ReadTask {
    repository: Arc<dyn BoardRepository>,
}

impl ReadTask {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        let _ = clock;
        Self { repository }
    }

    /// # Errors
    /// Always, until implemented.
    pub fn execute(
        &self,
        request: ReadTaskRequest,
    ) -> Result<crate::application::swarm::dto::TaskRow, BoardError> {
        let _ = (&self.repository, request);
        Err(BoardError::new("pending #2272"))
    }
}

#[cfg(test)]
#[path = "read_task_tests.rs"]
mod tests;
