//! RED STUB (#2271).
use std::sync::Arc;

use crate::application::swarm::dto::ActivateMemberRequest;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::BoardError;

pub struct ActivateMember {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ActivateMember {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// Not implemented yet.
    pub fn execute(&self, _request: ActivateMemberRequest) -> Result<(), BoardError> {
        let _stubbed = (&self.repository, &self.clock);
        Err(BoardError::new("not implemented (#2271)"))
    }
}

#[cfg(test)]
#[path = "activate_member_tests.rs"]
mod tests;
