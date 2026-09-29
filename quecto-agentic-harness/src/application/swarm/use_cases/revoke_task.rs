//! `Tasks.revoke(task_id, reason)` (#2275).
use std::sync::Arc;

use crate::application::swarm::dto::{RevokeTaskRequest, Revoked};
use crate::application::swarm::ports::{BoardEncoding, BoardRepository, Clock};
use crate::domain::swarm::BoardError;

pub struct RevokeTask {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    encoding: Arc<dyn BoardEncoding>,
}

impl RevokeTask {
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

    pub fn execute(&self, request: RevokeTaskRequest) -> Result<Revoked, BoardError> {
        let _ = (&self.repository, &self.clock, &self.encoding, request);
        Err(BoardError::new("revoke is not served yet"))
    }
}

#[cfg(test)]
#[path = "revoke_task_tests.rs"]
mod tests;
