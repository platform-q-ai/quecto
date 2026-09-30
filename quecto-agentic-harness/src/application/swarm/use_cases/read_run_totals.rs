//! `_run_totals` (#2313 review M1): the run's own totals, every member's
//! work, read once by the coordinator's harness when the run settles, for
//! the run-wide section of its `swarm_run_summary`. Rust-only: Python's
//! board has no such read.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use crate::application::swarm::board_operation::operation;
use crate::application::swarm::board_read_models::counts;
use crate::application::swarm::dto::RunTotalsView;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError};

/// Through the operation gate as a read (any member, a dead one
/// included), in one transaction: the tasks by state as `summary` counts
/// them, the messages by what became of them, each member's usage as
/// `usage_report` aggregates it (no request payload is read and no table
/// created, #2313 final review), the run's creation time and the board's
/// clock.
pub struct ReadRunTotals {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ReadRunTotals {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation refusal (no run, an unknown member), an edited
    /// record, or the store's.
    pub fn execute(&self, actor: &str) -> Result<RunTotalsView, BoardError> {
        let reading = Access {
            read_only: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            reading,
            |transaction, run| {
                let run_id = transaction
                    .run_row()?
                    .and_then(|row| row.get("id").and_then(Value::as_str).map(str::to_owned));
                Ok(RunTotalsView {
                    run_id,
                    task_count: transaction.task_count()?,
                    counts: counts(transaction, run.coordinator.as_deref())?,
                    messages: transaction.message_tally()?,
                    usage: transaction.member_usage()?,
                    created_at: transaction.created_at()?,
                    read_at: self.clock.now_seconds(),
                })
            },
        )
    }
}

impl OverRepository for ReadRunTotals {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "read_run_totals_tests.rs"]
mod tests;
