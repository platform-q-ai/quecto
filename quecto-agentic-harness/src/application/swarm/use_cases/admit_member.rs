//! RED STUB (#2271).
use std::sync::Arc;

use crate::application::swarm::dto::{AdmitMemberRequest, AdmittedMember};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::BoardError;

pub struct AdmitMember {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl AdmitMember {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// Not implemented yet.
    pub fn execute(&self, _request: AdmitMemberRequest) -> Result<AdmittedMember, BoardError> {
        let _stubbed = (&self.repository, &self.clock);
        Err(BoardError::new("not implemented (#2271)"))
    }
}

#[cfg(test)]
#[path = "admit_member_tests.rs"]
mod tests;
