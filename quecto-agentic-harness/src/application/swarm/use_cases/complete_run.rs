//! `Workbench.complete(revision)` (#2273).
use std::sync::Arc;

use crate::application::swarm::dto::CompleteRunRequest;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::BoardError;

pub struct CompleteRun {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl CompleteRun {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// Pending.
    pub fn execute(&self, request: CompleteRunRequest) -> Result<(), BoardError> {
        let _ = (&self.repository, &self.clock, request);
        Err(BoardError::new("pending #2273"))
    }
}

#[cfg(test)]
#[path = "complete_run_tests.rs"]
mod tests;
