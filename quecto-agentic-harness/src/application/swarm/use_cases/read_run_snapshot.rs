//! `Workbench._snapshot` (#2270): the run and its members, as the lifecycle
//! reads them.
use std::sync::Arc;

use super::OverRepository;
use crate::application::swarm::board_operation::read_operation;
use crate::application::swarm::dto::{RunSnapshotView, RunWatchView};
use crate::application::swarm::ports::{BoardRepository, BoardTransaction, Clock};
use crate::domain::swarm::{BoardError, RunRecord};

/// Through the operation gate as a read (`active=False, read_only=True`):
/// any member, a dead one included, reads it, and a run whose deadline has
/// come is ended as `budget-exhausted` first. Otherwise (#2338) it is one
/// read transaction, which takes no write lock: the run watch of every
/// member reads it, and it never contends with a writer.
pub struct ReadRunSnapshot {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ReadRunSnapshot {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation refusal (no run, an unknown member), or the store's.
    pub fn execute(&self, member: &str) -> Result<RunSnapshotView, BoardError> {
        read_operation(&*self.repository, &*self.clock, member, snapshot_of)
    }

    /// `_watch(since)` (#2338): the run watch's one call a tick. Through
    /// the same gate, in the same read transaction (the two IMMEDIATE ones
    /// when the deadline has come): the event cursor, and the snapshot
    /// unless the cursor is `since`, so an unchanged board answers only
    /// the cursor, and a changed one both, read together.
    ///
    /// # Errors
    /// As [`Self::execute`].
    pub fn watch(&self, member: &str, since: Option<i64>) -> Result<RunWatchView, BoardError> {
        debug_assert!(
            since.is_none_or(|since| since >= 0),
            "a cursor passed is one the board answered"
        );
        read_operation(
            &*self.repository,
            &*self.clock,
            member,
            |transaction, run| {
                let event_cursor = transaction.event_generation()?;
                debug_assert!(event_cursor >= 0, "an event cursor is never negative");
                let snapshot = match since == Some(event_cursor) {
                    true => None,
                    false => Some(snapshot_of(transaction, run)?),
                };
                Ok(RunWatchView {
                    event_cursor,
                    snapshot,
                })
            },
        )
    }
}

/// The run and its members, as `_snapshot` answers them.
fn snapshot_of(
    transaction: &dyn BoardTransaction,
    run: &RunRecord,
) -> Result<RunSnapshotView, BoardError> {
    let control_generation = transaction.control_generation()?;
    let members = transaction.members()?;
    Ok(RunSnapshotView {
        status: run.status.as_ref().map(|status| status.as_str().to_owned()),
        coordinator: run.coordinator.clone(),
        outcome: run.outcome.clone(),
        control_generation,
        deadline: run.deadline,
        members,
    })
}

impl OverRepository for ReadRunSnapshot {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "read_run_snapshot_tests.rs"]
mod tests;
