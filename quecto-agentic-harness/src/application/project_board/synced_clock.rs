//! The board server's clock gate (epic #2705): a clock that tells the time
//! only while the host affirms its clock is synchronised ([`TimeSync`]).
//!
//! - Synchronised: the clock's time, and that time is its own outside
//!   reference (the host keeps it to an outside time), so a change is
//!   bounded against it and its answer carries no clock note.
//! - Anything else: no time (`BoardClockError::Unsynced`, which the server
//!   refuses as `clock_unsynced` and rolls back) and no reference.
//!
//! The host is asked on each reading, so the gate follows it as it gains
//! or loses synchronisation.
use std::sync::Arc;
use std::time::Duration;

use super::ports::{BoardClock, BoardClockError, ClockSync, ReferenceStatus, TimeSync};
use crate::domain::project_board::value_objects::timestamp::Timestamp;

/// `clock`, gated on `sync`.
pub struct SyncedBoardClock {
    clock: Arc<dyn BoardClock>,
    sync: Arc<dyn TimeSync>,
}

impl SyncedBoardClock {
    pub fn new(clock: Arc<dyn BoardClock>, sync: Arc<dyn TimeSync>) -> Self {
        Self { clock, sync }
    }
}

impl BoardClock for SyncedBoardClock {
    fn now(&self) -> Result<Timestamp, BoardClockError> {
        match self.sync.sync() {
            ClockSync::Synced => self.clock.now(),
            ClockSync::Unsynced(why) => Err(BoardClockError::Unsynced(why)),
        }
    }

    /// The synchronised clock's own time: read after [`Self::now`], never
    /// before it.
    fn reference(&self) -> Option<Timestamp> {
        match self.sync.sync() {
            ClockSync::Synced => self.clock.now().ok(),
            ClockSync::Unsynced(_) => None,
        }
    }

    fn reference_status(&self) -> ReferenceStatus {
        match self.sync.sync() {
            ClockSync::Synced => ReferenceStatus::Referenced,
            ClockSync::Unsynced(why) => ReferenceStatus::NoReference(why),
        }
    }

    fn pause(&self, attempt: usize) {
        self.clock.pause(attempt);
    }

    fn elapsed(&self) -> Duration {
        self.clock.elapsed()
    }
}

#[cfg(test)]
#[path = "synced_clock_tests.rs"]
mod tests;
