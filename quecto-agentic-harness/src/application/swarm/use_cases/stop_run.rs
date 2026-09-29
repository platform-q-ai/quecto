//! STUB (#2273 red phase).
use std::sync::Arc;

use crate::application::swarm::dto::{ControlAnswer, StopRunRequest};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::BoardError;

pub struct StopRun {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl StopRun {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// Pending #2273.
    pub fn execute(&self, _request: StopRunRequest) -> Result<ControlAnswer, BoardError> {
        let _ = (&self.repository, &self.clock);
        Err(BoardError::new("pending #2273"))
    }
}

#[cfg(test)]
#[path = "stop_run_tests.rs"]
mod tests;
