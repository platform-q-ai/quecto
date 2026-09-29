//! `Workbench._resume_external()` (#2273): the supervisor outside the
//! swarm lifts a pause (#1729).
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use crate::application::swarm::board_control::{blockers, pause_started, paused_for, receipt};
use crate::application::swarm::board_operation::{detail, operation};
use crate::application::swarm::dto::{ControlAnswer, RunTransition};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError, RefusalKind, RunState};

/// Through the operation gate as the coordinator (the supervisor acts as
/// it). A running run answers its receipt as it stands; only a paused run
/// resumes, and only when nothing would pause it again at once. The
/// deadline moves on by the paused interval, the held outcome clears, the
/// run runs, and the event `resumed{paused_seconds,outcome}` names the
/// interval (the integer `0` unless positive, as Python's `max(0, …)`)
/// and the outcome the run held.
pub struct ResumeRunExternally {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ResumeRunExternally {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation refusal, `only a paused run may resume`, `resume
    /// would pause again at once: …`, a paused run without a pause record,
    /// or the store's.
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
                let transition = match run.status.as_ref().map(RunState::as_str) {
                    Some("running") => RunTransition::Unchanged,
                    Some("paused") => {
                        let now = self.clock.now_seconds();
                        let (elapsed, paused_seconds) =
                            paused_for(now, pause_started(transaction)?);
                        let deadline = run.deadline + elapsed;
                        let blocking = blockers(transaction, &*self.clock)?;
                        if !blocking.is_empty() {
                            return Err(BoardError::new(
                                RefusalKind::BudgetExhausted,
                                format!(
                                    "resume would pause again at once: {}",
                                    blocking.join("; ")
                                ),
                            ));
                        }
                        transaction.set_deadline(deadline)?;
                        transaction.clear_outcome()?;
                        transaction.set_outcome(&RunState::RUNNING)?;
                        let outcome = run.outcome.clone().map_or(Value::Null, Value::String);
                        // Python's `store.event` reads the clock again.
                        transaction.event(
                            actor,
                            self.clock.now_seconds(),
                            "resumed",
                            &detail([("paused_seconds", paused_seconds), ("outcome", outcome)]),
                        )?;
                        RunTransition::Applied
                    }
                    _ => {
                        return Err(BoardError::new(
                            RefusalKind::WrongState,
                            "only a paused run may resume",
                        ));
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

impl OverRepository for ResumeRunExternally {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "resume_run_externally_tests.rs"]
mod tests;
