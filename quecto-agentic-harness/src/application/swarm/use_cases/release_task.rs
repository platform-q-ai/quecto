//! Stub (#2272 red phase).
use std::sync::Arc;

use crate::application::swarm::dto::ReleaseTaskRequest;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::BoardError;

pub struct ReleaseTask {
    repository: Arc<dyn BoardRepository>,
}

impl ReleaseTask {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        let _ = clock;
        Self { repository }
    }

    /// # Errors
    /// Always, until implemented.
    pub fn execute(&self, request: ReleaseTaskRequest) -> Result<(), BoardError> {
        let _ = (&self.repository, request);
        Err(BoardError::new("pending #2272"))
    }
}

#[cfg(test)]
#[path = "release_task_tests.rs"]
mod tests;
