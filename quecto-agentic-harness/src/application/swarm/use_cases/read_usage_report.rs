//! STUB (#2273 red phase).
use std::sync::Arc;

use crate::application::swarm::dto::UsageReport;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::BoardError;

pub struct ReadUsageReport {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ReadUsageReport {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// Pending #2273.
    pub fn execute(&self, _actor: &str) -> Result<UsageReport, BoardError> {
        let _ = (&self.repository, &self.clock);
        Err(BoardError::new("pending #2273"))
    }
}

#[cfg(test)]
#[path = "read_usage_report_tests.rs"]
mod tests;
