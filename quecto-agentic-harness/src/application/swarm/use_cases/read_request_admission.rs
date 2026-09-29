//! `Workbench._request_admission()` (#2274): what a member's inference
//! admission reads before each model request, after the token budget
//! applies.
use std::sync::Arc;

use crate::application::swarm::board_control::current;
use crate::application::swarm::board_operation::operation;
use crate::application::swarm::board_usage::apply_usage_budget;
use crate::application::swarm::dto::{RequestAdmission, RunSnapshotView};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError};

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
                let effect = apply_usage_budget(transaction, &*self.clock, actor)?;
                let run = current(transaction)?;
                let members = transaction.members()?;
                let control_generation = transaction.control_generation()?;
                debug_assert!(control_generation >= 0, "an event id or 0");
                Ok(RequestAdmission {
                    effect,
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

#[cfg(test)]
#[path = "read_request_admission_tests.rs"]
mod tests;
