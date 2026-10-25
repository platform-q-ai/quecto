//! The board server's clock gate (epic #2705): a clock that tells the time
//! only while the host affirms its clock is synchronised ([`TimeSync`]).
//!
//! Only [`BoardClock::now`] is gated: unsynchronised, it is
//! `BoardClockError::Unsynced`, which the server refuses as
//! `clock_unsynced` and rolls back. Everything else is the wrapped clock's,
//! its reference included: a synchronised host is no outside reference, so
//! the server bounds no change against one (B7's behaviour, until B9).
//!
//! The host is asked on each reading of the time, and only then, so the
//! gate follows the host as it gains or loses synchronisation. An op may
//! read the time more than once (a retried write, a renewal's check before
//! its write), so the server gates each op's clock on [`HeldSync`]: the
//! host's word read once, as the op begins, and held for it.
use std::sync::Arc;
use std::time::Duration;

use super::ports::{BoardClock, BoardClockError, ClockSync, ReferenceStatus, TimeSync};
use crate::domain::project_board::value_objects::timestamp::Timestamp;

/// `clock`, its time gated on `sync`.
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

/// The host's word on its clock, read once and held: one op's.
pub struct HeldSync(ClockSync);

impl HeldSync {
    /// What `sync` says now.
    pub fn read(sync: &dyn TimeSync) -> Self {
        Self(sync.sync())
    }
}

impl TimeSync for HeldSync {
    fn sync(&self) -> ClockSync {
        self.0.clone()
    }
}

#[cfg(test)]
#[path = "synced_clock_tests.rs"]
mod tests;
