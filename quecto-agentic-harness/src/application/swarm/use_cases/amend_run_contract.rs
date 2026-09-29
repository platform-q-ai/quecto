//! Pending (#2273).
use std::sync::Arc;

use crate::application::swarm::dto::AmendRunContractRequest;
use crate::application::swarm::ports::{BoardEncoding, BoardRepository, Clock};
use crate::domain::swarm::BoardError;

pub struct AmendRunContract {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    encoding: Arc<dyn BoardEncoding>,
}

impl AmendRunContract {
    pub fn new(
        repository: Arc<dyn BoardRepository>,
        clock: Arc<dyn Clock + Send + Sync>,
        encoding: Arc<dyn BoardEncoding>,
    ) -> Self {
        Self {
            repository,
            clock,
            encoding,
        }
    }

    /// # Errors
    /// Pending.
    pub fn execute(&self, request: AmendRunContractRequest) -> Result<(), BoardError> {
        let _ = (&self.repository, &self.clock, &self.encoding, request);
        Err(BoardError::new("pending #2273"))
    }
}

#[cfg(test)]
#[path = "amend_run_contract_tests.rs"]
mod tests;
