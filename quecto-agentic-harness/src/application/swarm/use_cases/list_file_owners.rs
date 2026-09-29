//! `Tasks.file_owners(offset=0, limit=50)` (#2275).
use std::sync::Arc;

use crate::application::swarm::dto::{FileRow, ListFileOwnersRequest};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::BoardError;

pub struct ListFileOwners {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ListFileOwners {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    pub fn execute(&self, request: ListFileOwnersRequest) -> Result<Vec<FileRow>, BoardError> {
        let _ = (&self.repository, &self.clock, request);
        Err(BoardError::new("file_owners is not served yet"))
    }
}

#[cfg(test)]
#[path = "list_file_owners_tests.rs"]
mod tests;
