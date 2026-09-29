//! STUB (#2273 red phase).
use std::sync::Arc;

use crate::application::swarm::dto::ControlReceipt;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::BoardError;

pub struct ReadControlStatus {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ReadControlStatus {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// Pending #2273.
    pub fn execute(&self, _actor: &str) -> Result<ControlReceipt, BoardError> {
        let _ = (&self.repository, &self.clock);
        Err(BoardError::new("pending #2273"))
    }
}

#[cfg(test)]
#[path = "read_control_status_tests.rs"]
mod tests;
