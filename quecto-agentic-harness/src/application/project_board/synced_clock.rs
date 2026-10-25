//! The board server's clock gate (epic #2705): a clock that tells the time
//! only while the host says its clock is synchronised.
use std::sync::Arc;
use std::time::Duration;

use super::ports::{BoardClock, BoardClockError, ReferenceStatus, TimeSync};
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
        let _ = &self.sync;
        self.clock.now()
    }

    fn reference(&self) -> Option<Timestamp> {
        self.clock.reference()
    }

    fn reference_status(&self) -> ReferenceStatus {
        self.clock.reference_status()
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
