//! RED STUB (#2271).
use std::sync::Arc;

use crate::application::swarm::dto::RecordMemberLaunchRequest;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::BoardError;

pub struct RecordMemberLaunch {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl RecordMemberLaunch {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// Not implemented yet.
    pub fn execute(&self, _request: RecordMemberLaunchRequest) -> Result<(), BoardError> {
        let _stubbed = (&self.repository, &self.clock);
        Err(BoardError::new("not implemented (#2271)"))
    }
}

#[cfg(test)]
#[path = "record_member_launch_tests.rs"]
mod tests;
