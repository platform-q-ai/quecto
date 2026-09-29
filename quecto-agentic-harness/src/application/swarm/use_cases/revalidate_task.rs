//! Pending (#2273).
use std::sync::Arc;

use crate::application::swarm::dto::RevalidateTaskRequest;
use crate::application::swarm::ports::{BoardEncoding, BoardRepository, Clock};
use crate::domain::swarm::BoardError;

pub struct RevalidateTask {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    encoding: Arc<dyn BoardEncoding>,
}

impl RevalidateTask {
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
    /// Pending.
    pub fn execute(&self, request: RevalidateTaskRequest) -> Result<(), BoardError> {
        let _ = (&self.repository, &self.clock, &self.encoding, request);
        Err(BoardError::new("pending #2273"))
    }
}

#[cfg(test)]
#[path = "revalidate_task_tests.rs"]
mod tests;
