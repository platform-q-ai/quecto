//! `Workbench._extend_deadline(seconds)` (#2273): the supervisor grants a
//! run wall-clock budget (#1729).
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use crate::application::swarm::board_control::{pause_started, receipt};
use crate::application::swarm::board_operation::{detail, operation, seconds};
use crate::application::swarm::dto::{ControlAnswer, ExtendRunDeadlineRequest, RunTransition};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::policy::MAX_EXTENSION_SECONDS;
use crate::domain::swarm::{Access, BoardError, RefusalKind, RunState, validate_extension};

/// The seconds are validated before the operation gate, which admits only
/// the coordinator. A running or paused run's deadline moves to its base
/// plus the seconds, where a paused run's base is the later of its
/// deadline and its pause start (a resume adds the paused interval back,
/// so the grant counts from then). The new deadline may be at most seven
/// days ahead of the clock; the event `extended{seconds,deadline}` records
/// both.
pub struct ExtendRunDeadline {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ExtendRunDeadline {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// `deadline extension must be 1..604800 seconds`, an authorisation
    /// refusal, `run is …; nothing to extend`, `deadline may be at most
    /// seven days ahead, as at creation`, or the store's.
    pub fn execute(&self, request: ExtendRunDeadlineRequest) -> Result<ControlAnswer, BoardError> {
        let granted = validate_extension(&request.seconds)?;
        let actor = request.actor.as_str();
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
                let base = match run.status.as_ref().map(RunState::as_str) {
                    Some("running") => run.deadline,
                    // `max(deadline, started)`: on a tie either is the
                    // same value, and neither is NaN (SQLite stores none,
                    // and `pause_started` admits only a finite start).
                    Some("paused") => run.deadline.max(pause_started(transaction)?),
                    status => {
                        return Err(BoardError::new(
                            RefusalKind::NotRunning,
                            format!("run is {}; nothing to extend", status.unwrap_or("None")),
                        ));
                    }
                };
                debug_assert!(
                    (1..=MAX_EXTENSION_SECONDS).contains(&granted),
                    "a validated extension, exact as a float: {granted}"
                );
                let deadline = base + granted as f64;
                let horizon = self.clock.now_seconds() + MAX_EXTENSION_SECONDS as f64;
                if deadline > horizon {
                    return Err(BoardError::new(
                        RefusalKind::Invalid,
                        "deadline may be at most seven days ahead, as at creation",
                    ));
                }
                transaction.set_deadline(deadline)?;
                // Python's `store.event` reads the clock again.
                transaction.event(
                    actor,
                    self.clock.now_seconds(),
                    "extended",
                    &detail([
                        ("seconds", Value::from(granted)),
                        ("deadline", seconds(deadline)),
                    ]),
                )?;
                Ok(ControlAnswer {
                    transition: RunTransition::Applied,
                    receipt: receipt(transaction, &*self.clock)?,
                })
            },
        )
    }
}

impl OverRepository for ExtendRunDeadline {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "extend_run_deadline_tests.rs"]
mod tests;
