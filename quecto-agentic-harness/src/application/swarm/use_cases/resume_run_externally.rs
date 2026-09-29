//! STUB (#2273 red phase).
use std::sync::Arc;

use crate::application::swarm::dto::ControlAnswer;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::BoardError;

pub struct ResumeRunExternally {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ResumeRunExternally {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// Pending #2273.
    pub fn execute(&self, _actor: &str) -> Result<ControlAnswer, BoardError> {
        let _ = (&self.repository, &self.clock);
        Err(BoardError::new("pending #2273"))
    }
}

#[cfg(test)]
#[path = "resume_run_externally_tests.rs"]
mod tests;
