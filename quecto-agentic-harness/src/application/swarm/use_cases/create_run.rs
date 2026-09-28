//! `Workbench.create` (#2270): red-phase skeleton.
use std::sync::Arc;

use crate::application::swarm::dto::{CreateRunRequest, CreatedRun};
use crate::application::swarm::ports::{BoardEncoding, BoardRepository, Clock, IdSource};
use crate::domain::swarm::BoardError;

pub struct CreateRun {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    ids: Arc<dyn IdSource>,
    encoding: Arc<dyn BoardEncoding>,
}

impl CreateRun {
    pub fn new(
        repository: Arc<dyn BoardRepository>,
        clock: Arc<dyn Clock + Send + Sync>,
        ids: Arc<dyn IdSource>,
        encoding: Arc<dyn BoardEncoding>,
    ) -> Self {
        Self {
            repository,
            clock,
            ids,
            encoding,
        }
    }

    /// # Errors
    /// Not implemented yet.
    pub fn execute(&self, request: CreateRunRequest) -> Result<CreatedRun, BoardError> {
        let _ = (
            &self.repository,
            &self.clock,
            &self.ids,
            &self.encoding,
            request,
        );
        Err(BoardError::new("not implemented yet (#2270)"))
    }
}

#[cfg(test)]
#[path = "create_run_tests.rs"]
mod tests;
