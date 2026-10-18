//! `Workbench._request_admission()` (#2274): what a member's inference
//! admission reads before each model request. No token budget applies:
//! usage never pauses a run (owner decision, 2026-10-14).
use std::sync::Arc;

use super::OverRepository;

use crate::application::swarm::board_control::current;
use crate::application::swarm::board_operation::operation;
use crate::application::swarm::dto::{RequestAdmission, RunSnapshotView};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{services::policy::Access, value_objects::board_error::BoardError};

/// Through the operation gate for reading (any member, a dead one
/// included): the run as it stands (a passed deadline has paused it
/// holding `budget-exhausted`), its members and the control generation.
pub struct ReadRequestAdmission {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ReadRequestAdmission {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation refusal, or the store's.
    pub fn execute(&self, actor: &str) -> Result<RequestAdmission, BoardError> {
        let reading = Access {
            read_only: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            reading,
            |transaction, _| {
                // The usage tables are made as Python's budget read made
                // them, so the board file is the one it wrote; no budget is
                // read (owner decision, 2026-10-14), so none can refuse it.
                transaction.usage_tables()?;
                let run = current(transaction)?;
                let members = transaction.members()?;
                let control_generation = transaction.control_generation()?;
                debug_assert!(control_generation >= 0, "an event id or 0");
                Ok(RequestAdmission {
                    view: RunSnapshotView {
                        status: run.status.as_ref().map(|status| status.as_str().to_owned()),
                        coordinator: run.coordinator,
                        outcome: run.outcome,
                        control_generation,
                        deadline: run.deadline,
                        members,
                    },
                })
            },
        )
    }
}

impl OverRepository for ReadRequestAdmission {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "read_request_admission_tests.rs"]
mod tests;
