//! STUB (#2273 red phase).
use std::sync::Arc;

use crate::application::swarm::dto::{ControlAnswer, ExtendRunDeadlineRequest};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::BoardError;

pub struct ExtendRunDeadline {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ExtendRunDeadline {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// Pending #2273.
    pub fn execute(&self, _request: ExtendRunDeadlineRequest) -> Result<ControlAnswer, BoardError> {
        let _ = (&self.repository, &self.clock);
        Err(BoardError::new("pending #2273"))
    }
}

#[cfg(test)]
#[path = "extend_run_deadline_tests.rs"]
mod tests;
