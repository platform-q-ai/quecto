//! `Workbench._lose_coordinator` (#2277, #1924): the supervising session
//! outside the container lost this coordinator's socket.
use std::sync::Arc;

use super::OverRepository;
use crate::application::swarm::board_loss::{
    LOST_HARNESS, SCOPE_UNKNOWN, end_by_loss, holds_outcome,
};
use crate::application::swarm::board_operation::{detail, operation, text};
use crate::application::swarm::dto::{CoordinatorLoss, LoseCoordinatorRequest};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError, RefusalKind, RunState};

/// In one operation (`active=False`), so the expiry and any concurrent end
/// are observed first: the setup placeholder, a running run, and a pause
/// holding no outcome are lost as a lost harness (a `scope_unknown` event
/// naming the caller, then the end by loss); a run that holds an outcome
/// or has ended is left alone. Answers the run's columns after the op and
/// whether a loss was recorded.
pub struct LoseCoordinator {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl LoseCoordinator {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation refusal, or the store's.
    pub fn execute(&self, request: LoseCoordinatorRequest) -> Result<CoordinatorLoss, BoardError> {
        let actor = request.actor.as_str();
        let clock = &*self.clock;
        operation(
            &*self.repository,
            clock,
            actor,
            Access::default(),
            |transaction, run| {
                let losable = match run.status.as_ref().map(RunState::as_str) {
                    Some("setup" | "running") => true,
                    Some("paused") => !holds_outcome(run),
                    _ => false,
                };
                if losable {
                    transaction.event(
                        actor,
                        clock.now_seconds(),
                        "scope_unknown",
                        &detail([("member", text(actor)), ("reason", text(SCOPE_UNKNOWN))]),
                    )?;
                    let ended = end_by_loss(transaction, clock, actor, run, LOST_HARNESS)?;
                    debug_assert!(ended, "a losable run ends by the loss");
                }
                let row = transaction.run_status()?.ok_or_else(|| {
                    BoardError::new(RefusalKind::RunMissing, "coordination run missing")
                })?;
                Ok(CoordinatorLoss {
                    run: row,
                    lost: losable,
                })
            },
        )
    }
}

impl OverRepository for LoseCoordinator {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "lose_coordinator_tests.rs"]
mod tests;
