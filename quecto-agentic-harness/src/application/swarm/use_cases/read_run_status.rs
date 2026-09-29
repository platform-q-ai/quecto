//! `Workbench._status` (#2270): has a run been created in this container?
use std::sync::Arc;

use serde_json::Value;

use crate::application::swarm::board_operation::atomic;
use crate::application::swarm::dto::RunStatusView;
use crate::application::swarm::ports::BoardRepository;
use crate::domain::swarm::BoardError;

/// Membership-free, so it runs in one plain transaction rather than the
/// operation gate: the harness and the supervising session read it before
/// joining, or after every member is gone. The bootstrap placeholder
/// carries deadline 0 and `create` requires a future one; with no run at
/// all the status is `setup` and the deadline Python's integer `0`. It
/// reads only the columns Python's `_status` selects, each as stored.
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
            let row = transaction.run_status()?;
            let coordinator = row.as_ref().and_then(|row| row.coordinator.as_deref());
            let counts = transaction.claim_counts(coordinator)?;
            Ok(match row {
                Some(row) => RunStatusView {
                    counts,
                    id: row.id,
                    status: row.status,
                    deadline: row.deadline,
                    coordinator: row.coordinator,
                    outcome: row.outcome,
                },
                None => RunStatusView {
                    counts,
                    id: None,
                    status: Some("setup".to_owned()),
                    deadline: Value::from(0),
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
