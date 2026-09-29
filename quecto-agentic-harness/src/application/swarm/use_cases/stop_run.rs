//! `Workbench.stop(status, reason)` (#2273): the coordinator ends the run.
//! Every status but `cancelled` is a resumable pause holding that outcome
//! for the supervisor outside the swarm (#1729); cancellation is terminal
//! at once.
use std::sync::Arc;

use super::OverRepository;
use crate::application::swarm::board_control::receipt;
use crate::application::swarm::board_operation::{detail, end, operation, text};
use crate::application::swarm::dto::{ControlAnswer, RunTransition, StopRunRequest};
use crate::application::swarm::ports::{BoardRepository, BoardTransaction, Clock};
use crate::domain::swarm::validation::TEXT_MAX_BYTES;
use crate::domain::swarm::{
    Access, BoardError, RefusalKind, RunRecord, RunState, STOP_STATUSES, bounded, run_already,
    run_already_held,
};

/// The reason is bounded and the status must be one of `STOP_STATUSES`
/// before the operation gate, which admits only the coordinator.
///
/// - `cancelled`: a cancelled run answers as it stands; a running or
///   paused run drops any held outcome, is cancelled and records
///   `stop{status,reason}`; a setup placeholder and any other run are
///   refused.
/// - any other status: a paused run already holding it answers as it
///   stands; a running run pauses holding it (`stop`, then `paused`, as
///   the gate's own end); any other run is refused.
pub struct StopRun {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl StopRun {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// The reason's bound, `invalid non-success outcome`, an authorisation
    /// refusal, `run not created yet; …`, `run already …`, or the store's.
    pub fn execute(&self, request: StopRunRequest) -> Result<ControlAnswer, BoardError> {
        let reason = bounded(&request.reason, "stop reason", TEXT_MAX_BYTES)?;
        let status = match request.status.as_str() {
            Some(status) if STOP_STATUSES.contains(&status) => status,
            _ => {
                return Err(BoardError::new(
                    RefusalKind::Invalid,
                    "invalid non-success outcome",
                ));
            }
        };
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
                let transition = match status {
                    "cancelled" => self.cancel(transaction, run, actor, reason)?,
                    _ => self.end(transaction, run, actor, status, reason)?,
                };
                Ok(ControlAnswer {
                    transition,
                    receipt: receipt(transaction, &*self.clock)?,
                })
            },
        )
    }

    fn cancel(
        &self,
        transaction: &dyn BoardTransaction,
        run: &RunRecord,
        actor: &str,
        reason: &str,
    ) -> Result<RunTransition, BoardError> {
        match run.status.as_ref().map(RunState::as_str) {
            Some("cancelled") => Ok(RunTransition::Unchanged),
            Some("setup") => Err(BoardError::new(
                RefusalKind::RunMissing,
                "run not created yet; nothing to cancel. To start one: swarm op=create",
            )),
            Some("running" | "paused") => {
                transaction.clear_outcome()?;
                transaction.set_outcome(&RunState::CANCELLED)?;
                transaction.event(
                    actor,
                    self.clock.now_seconds(),
                    "stop",
                    &detail([("status", text("cancelled")), ("reason", text(reason))]),
                )?;
                Ok(RunTransition::Applied)
            }
            _ => Err(run_already(run)),
        }
    }

    fn end(
        &self,
        transaction: &dyn BoardTransaction,
        run: &RunRecord,
        actor: &str,
        status: &str,
        reason: &str,
    ) -> Result<RunTransition, BoardError> {
        match (
            run.status.as_ref().map(RunState::as_str),
            run.outcome.as_deref(),
        ) {
            (Some("paused"), Some(held)) if held == status => Ok(RunTransition::Unchanged),
            (Some("running"), _) => {
                end(transaction, &*self.clock, actor, status, reason)?;
                Ok(RunTransition::Applied)
            }
            _ => Err(run_already_held(run)),
        }
    }
}

impl OverRepository for StopRun {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "stop_run_tests.rs"]
mod tests;
