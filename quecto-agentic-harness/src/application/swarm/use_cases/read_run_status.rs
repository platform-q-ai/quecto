//! `Workbench._status` (#2270): has a run been created in this container?
use std::sync::Arc;

use crate::application::swarm::board_operation::atomic;
use crate::application::swarm::dto::{RunStatusView, StatusDeadline};
use crate::application::swarm::ports::BoardRepository;
use crate::domain::swarm::BoardError;

/// Membership-free, so it runs in one plain transaction rather than the
/// operation gate: the harness and the supervising session read it before
/// joining, or after every member is gone. The bootstrap placeholder
/// carries deadline 0 and `create` requires a future one; with no run at
/// all the status is `setup` and the deadline Python's integer `0`.
pub struct ReadRunStatus {
    repository: Arc<dyn BoardRepository>,
}

impl ReadRunStatus {
    pub fn new(repository: Arc<dyn BoardRepository>) -> Self {
        Self { repository }
    }

    /// # Errors
    /// The store's refusal (a missing board, contention).
    pub fn execute(&self) -> Result<RunStatusView, BoardError> {
        atomic(&*self.repository, false, |transaction| {
            let run = transaction.run()?;
            let id = transaction.run_id()?;
            let coordinator = run.as_ref().map(|run| run.coordinator.clone());
            let counts = transaction.claim_counts(coordinator.as_deref())?;
            Ok(match run {
                Some(run) => RunStatusView {
                    counts,
                    id,
                    status: run.status.as_str().to_owned(),
                    deadline: StatusDeadline::Stored(run.deadline),
                    coordinator,
                    outcome: run.outcome,
                },
                None => RunStatusView {
                    counts,
                    id: None,
                    status: "setup".to_owned(),
                    deadline: StatusDeadline::NoRun,
                    coordinator: None,
                    outcome: None,
                },
            })
        })
    }
}

#[cfg(test)]
#[path = "read_run_status_tests.rs"]
mod tests;
