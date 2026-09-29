//! `Workbench._close()` (#2273): the supervisor makes the outcome a paused
//! run holds terminal (#1729).
use std::sync::Arc;

use serde_json::Value;

use crate::application::swarm::board_control::receipt;
use crate::application::swarm::board_operation::{detail, operation, text};
use crate::application::swarm::dto::{ControlAnswer, RunTransition};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError, PROPOSED_OUTCOMES, RunState};

/// Through the operation gate as the coordinator: only a paused run
/// holding a proposed outcome (`PROPOSED_OUTCOMES`) closes. Its status
/// becomes that outcome (the outcome and reason stay), and the event
/// `closed{status,reason}` records both.
pub struct CloseRun {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl CloseRun {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation refusal, `run is … without a proposed outcome;
    /// resume it or cancel the run`, or the store's.
    pub fn execute(&self, actor: &str) -> Result<ControlAnswer, BoardError> {
        let coordinating = Access {
            coordinator: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            coordinating,
            |transaction, run| {
                let status = run.status.as_ref().map(RunState::as_str);
                let held = match (status, run.outcome.as_deref()) {
                    (Some("paused"), Some(outcome)) if PROPOSED_OUTCOMES.contains(&outcome) => {
                        outcome
                    }
                    _ => {
                        return Err(BoardError::new(format!(
                            "run is {} without a proposed outcome; resume it or cancel the run",
                            status.unwrap_or("None")
                        )));
                    }
                };
                transaction.set_outcome(&RunState::new(held))?;
                let reason = run
                    .outcome_reason
                    .clone()
                    .map_or(Value::Null, Value::String);
                transaction.event(
                    actor,
                    self.clock.now_seconds(),
                    "closed",
                    &detail([("status", text(held)), ("reason", reason)]),
                )?;
                Ok(ControlAnswer {
                    transition: RunTransition::Applied,
                    receipt: receipt(transaction, &*self.clock)?,
                })
            },
        )
    }
}

#[cfg(test)]
#[path = "close_run_tests.rs"]
mod tests;
