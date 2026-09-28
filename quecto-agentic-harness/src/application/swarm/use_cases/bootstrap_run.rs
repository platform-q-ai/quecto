//! The first half of `Workbench._bootstrap` (#2270): red-phase skeleton.
use std::sync::Arc;

use crate::application::swarm::dto::{BootstrapRunRequest, Bootstrapped};
use crate::application::swarm::ports::{BoardRepository, Clock, IdSource};
use crate::domain::swarm::BoardError;

pub struct BootstrapRun {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    ids: Arc<dyn IdSource>,
}

impl BootstrapRun {
    pub fn new(
        repository: Arc<dyn BoardRepository>,
        clock: Arc<dyn Clock + Send + Sync>,
        ids: Arc<dyn IdSource>,
    ) -> Self {
        Self {
            repository,
            clock,
            ids,
        }
    }

    /// # Errors
    /// Not implemented yet.
    pub fn execute(&self, request: BootstrapRunRequest) -> Result<Bootstrapped, BoardError> {
        let _ = (&self.repository, &self.clock, &self.ids, request);
        Err(BoardError::new("not implemented yet (#2270)"))
    }
}

#[cfg(test)]
#[path = "bootstrap_run_tests.rs"]
mod tests;
