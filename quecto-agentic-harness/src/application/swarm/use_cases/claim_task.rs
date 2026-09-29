//! Stub (#2272 red phase).
use std::sync::Arc;

use crate::application::swarm::dto::ClaimTaskRequest;
use crate::application::swarm::ports::{BoardRepository, Clock, IdSource};
use crate::domain::swarm::BoardError;

pub struct ClaimTask {
    repository: Arc<dyn BoardRepository>,
}

impl ClaimTask {
    pub fn new(
        repository: Arc<dyn BoardRepository>,
        clock: Arc<dyn Clock + Send + Sync>,
        ids: Arc<dyn IdSource>,
    ) -> Self {
        let _ = (clock, ids);
        Self { repository }
    }

    /// # Errors
    /// Always, until implemented.
    pub fn execute(
        &self,
        request: ClaimTaskRequest,
    ) -> Result<crate::application::swarm::dto::TaskRow, BoardError> {
        let _ = (&self.repository, request);
        Err(BoardError::new("pending #2272"))
    }
}

#[cfg(test)]
#[path = "claim_task_tests.rs"]
mod tests;
