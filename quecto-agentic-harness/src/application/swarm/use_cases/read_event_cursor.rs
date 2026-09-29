//! The board's event cursor (#2279): `SELECT coalesce(max(id),0) FROM
//! events`, the generation a structured op compares before and after its
//! call to decide whether the board changed (epic #2265 P5). Rust-only:
//! the dispatcher serves it as the internal method `_event_cursor`, never
//! as a member op.
use std::sync::Arc;

use super::OverRepository;
use crate::application::swarm::board_operation::atomic;
use crate::application::swarm::ports::BoardRepository;
use crate::domain::swarm::BoardError;

/// Membership-free, in one plain transaction (as `_status`), so the tool
/// reads it around any member's call without the operation gate: a read
/// never expires a run or writes an event, so reading the cursor cannot
/// move it.
pub struct ReadEventCursor {
    repository: Arc<dyn BoardRepository>,
}

impl ReadEventCursor {
    pub fn new(repository: Arc<dyn BoardRepository>) -> Self {
        Self { repository }
    }

    /// The id of the latest event, `0` for a board without events.
    ///
    /// # Errors
    /// The store's refusal (a missing board, contention).
    pub fn execute(&self) -> Result<i64, BoardError> {
        let cursor = atomic(&*self.repository, false, |transaction| {
            transaction.event_generation()
        })?;
        // SQLite's rowids start at 1, and `coalesce` answers 0 for none.
        debug_assert!(cursor >= 0, "an event cursor is never negative");
        Ok(cursor)
    }
}

impl OverRepository for ReadEventCursor {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self { repository }
    }
}

#[cfg(test)]
#[path = "read_event_cursor_tests.rs"]
mod tests;
