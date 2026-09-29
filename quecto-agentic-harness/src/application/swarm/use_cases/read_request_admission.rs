//! `Workbench._request_admission()` (#2274): what a member's inference
//! admission reads before each model request, after the token budget
//! applies.
use std::sync::Arc;

use crate::application::swarm::dto::RequestAdmission;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::BoardError;

/// Through the operation gate for reading (any member, a dead one
/// included): the budget applies (a warning once, a running run paused
/// holding `budget-exhausted`), then the run as it now stands, its
/// members and the control generation.
pub struct ReadRequestAdmission {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ReadRequestAdmission {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation refusal, an edited budget, or the store's.
    pub fn execute(&self, actor: &str) -> Result<RequestAdmission, BoardError> {
        let _ = (&self.repository, &self.clock, actor);
        Err(BoardError::new("pending #2274"))
    }
}

#[cfg(test)]
#[path = "read_request_admission_tests.rs"]
mod tests;
