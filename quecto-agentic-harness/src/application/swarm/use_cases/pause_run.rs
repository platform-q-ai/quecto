//! `Workbench.pause(reason)` (#2273): the coordinator pauses a running run.
use std::sync::Arc;

use super::OverRepository;
use crate::application::swarm::board_control::receipt;
use crate::application::swarm::board_operation::{detail, operation, seconds, text};
use crate::application::swarm::dto::{ControlAnswer, PauseRunRequest, RunTransition};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::validation::TEXT_MAX_BYTES;
use crate::domain::swarm::{Access, BoardError, RunState, authorize, bounded};

/// The reason is bounded before the operation gate, which admits only the
/// coordinator. A paused run answers its receipt as it stands; any other
/// run must be running, and pauses (its outcome and reason untouched)
/// with the event `paused{reason,started}`, `started` the clock's reading.
pub struct PauseRun {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl PauseRun {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// The reason's bound, an authorisation refusal (a run that is not
    /// running included), or the store's.
    pub fn execute(&self, request: PauseRunRequest) -> Result<ControlAnswer, BoardError> {
        let reason = bounded(&request.reason, "pause reason", TEXT_MAX_BYTES)?;
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
                let transition = match run.status.as_ref().map(RunState::as_str) {
                    Some("paused") => RunTransition::Unchanged,
                    _ => {
                        let running = Access {
                            active: true,
                            ..Access::default()
                        };
                        let member = transaction.member(actor)?;
                        authorize(Some(run), actor, member.as_ref(), running)?;
                        transaction.set_outcome(&RunState::PAUSED)?;
                        // Python reads the clock for `started` and again,
                        // inside `store.event`, for the event's time.
                        let started = seconds(self.clock.now_seconds());
                        transaction.event(
                            actor,
                            self.clock.now_seconds(),
                            "paused",
                            &detail([("reason", text(reason)), ("started", started)]),
                        )?;
                        RunTransition::Applied
                    }
                };
                Ok(ControlAnswer {
                    transition,
                    receipt: receipt(transaction, &*self.clock)?,
                })
            },
        )
    }
}

impl OverRepository for PauseRun {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "pause_run_tests.rs"]
mod tests;
