//! `Workbench.complete(revision)` (#2273): the coordinator proposes
//! success. Like every proposed outcome it ends the run as a resumable
//! pause (#1729); only the supervisor outside the swarm closes it.
use std::sync::Arc;

use crate::application::swarm::board_completion::{criteria, evidence_rows, task_record};
use crate::application::swarm::board_operation::{detail, end, operation};
use crate::application::swarm::dto::CompleteRunRequest;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError, RunState, completion, completion_revision};

/// Through the operation gate for the coordinator (a running run): the
/// completion state is read and judged by the domain's `completion` (the
/// revision, accepted evidence of each criterion's kind at it, settled
/// work and reservations, current task evidence, in that order). Success
/// records `completed{revision}` and then ends the run holding
/// `succeeded` for the reason `completed at {revision}` (`stop`, then
/// `paused`).
pub struct CompleteRun {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl CompleteRun {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation or budget refusal, a completion refusal with
    /// Python's text, criteria the board never writes, or the store's.
    pub fn execute(&self, request: CompleteRunRequest) -> Result<(), BoardError> {
        let actor = request.actor.as_str();
        let coordinating = Access {
            active: true,
            coordinator: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            coordinating,
            |transaction, _| {
                let state = transaction.completion_state()?;
                // Python's completion policy checks the revision first, even if
                // stored criteria were edited outside the board. The policy
                // repeats this guard so its own callers keep the invariant.
                completion_revision(&request.revision)?;
                let tasks: Vec<_> = state.tasks.iter().map(task_record).collect();
                let outcome = completion(
                    &criteria(&state.criteria)?,
                    &evidence_rows(&state.evidence),
                    &tasks,
                    state.has_reservations,
                    &request.revision,
                )?;
                // `completion` refuses anything but a revision with content.
                let revision = request.revision.as_str().ok_or_else(|| {
                    BoardError::new("completion accepted a revision that is not text")
                })?;
                debug_assert_eq!(outcome, RunState::SUCCEEDED, "completion only succeeds");
                transaction.event(
                    actor,
                    self.clock.now_seconds(),
                    "completed",
                    &detail([("revision", request.revision.clone())]),
                )?;
                end(
                    transaction,
                    &*self.clock,
                    actor,
                    outcome.as_str(),
                    &format!("completed at {revision}"),
                )
            },
        )
    }
}

#[cfg(test)]
#[path = "complete_run_tests.rs"]
mod tests;
