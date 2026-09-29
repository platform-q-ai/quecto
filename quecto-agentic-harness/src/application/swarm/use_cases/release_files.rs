//! `Tasks.release_files(task_id, token, reservation)` (#2275).
use std::sync::Arc;

use crate::application::swarm::dto::ReleaseFilesRequest;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::BoardError;

pub struct ReleaseFiles {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ReleaseFiles {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    pub fn execute(&self, request: ReleaseFilesRequest) -> Result<(), BoardError> {
        let _ = (&self.repository, &self.clock, request);
        Err(BoardError::new("release_files is not served yet"))
    }
}

#[cfg(test)]
#[path = "release_files_tests.rs"]
mod tests;
