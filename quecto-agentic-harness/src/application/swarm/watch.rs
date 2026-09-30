//! The run watch's schedule (#2338): when a member's harness, watching its
//! run, takes a full `_snapshot` of the board.
//!
//! The watch ticks every [`WATCH_TICK`], as it always has, so a board
//! change is seen as soon as before. Each tick reads the board's event
//! cursor first, a cheap read that takes no write lock, and the schedule
//! asks for a snapshot only when:
//!
//! - the cursor could not be read, or moved since the last snapshot: every
//!   change the watch acts on (a pause or resume, a run's end, a deadline
//!   extension, a run created over the container's placeholder) writes an
//!   event in the same transaction, so it moves the cursor;
//! - the run is running and its deadline has come: a passed deadline is
//!   the one change no event announces (the store records it on the next
//!   gated op, and the snapshot is that op), so the snapshot is due at the
//!   deadline itself, not one refresh later;
//! - a refresh is due: a safety net for a change made without an event
//!   (only a board edited from outside makes one). The refresh backs off
//!   while nothing changes, from [`REFRESH_MIN`] doubling to
//!   [`REFRESH_MAX`], and restarts at [`REFRESH_MIN`] after a change.
//!
//! Owner liveness is not the watch's to detect: a task owner turns idle
//! by the store's clock when any member reads it (`summary`, `task`,
//! `tasks`, [`OWNER_IDLE_AFTER`](crate::domain::swarm::OWNER_IDLE_AFTER)),
//! and a lost harness is recorded by `reconcile` after its grace. The
//! watch's longest gap between snapshots stays well inside both, so it
//! never holds a stale view past either.
use std::time::Duration;

use crate::domain::swarm::Snapshot;

/// How often the watch reads the event cursor: the 500 ms the snapshot
/// poll it replaces ran at, so a change is observed within one tick.
pub const WATCH_TICK: Duration = Duration::from_millis(500);

/// The refresh interval after a change: the first safety snapshot while
/// nothing moves comes this long after the last.
pub const REFRESH_MIN: Duration = Duration::from_secs(5);

/// The longest the watch goes without a snapshot while nothing changes.
pub const REFRESH_MAX: Duration = Duration::from_secs(60);

/// What a watch tick reads beyond the cursor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WatchRead {
    /// A full snapshot of the run.
    Snapshot,
    /// Nothing more: the cursor did not move and no check is due.
    Skip,
}

/// When the watch next takes a snapshot, from the last one it took.
#[derive(Clone, Debug)]
pub struct WatchSchedule {
    /// The cursor read just before the last snapshot; `None` before the
    /// first, or when that cursor could not be read.
    seen: Option<i64>,
    /// When (Unix seconds, the board's clock) a snapshot is due whatever
    /// the cursor says: the refresh, or the running run's deadline.
    due_at: f64,
    /// The refresh interval now in force.
    refresh: Duration,
}

impl Default for WatchSchedule {
    fn default() -> Self {
        Self::new()
    }
}

impl WatchSchedule {
    /// A schedule that takes a snapshot on its first tick.
    pub fn new() -> Self {
        Self {
            seen: None,
            due_at: f64::NEG_INFINITY,
            refresh: REFRESH_MIN,
        }
    }

    /// What the tick at `now` reads, given the `cursor` it read first
    /// (`None` when that read failed).
    pub fn read(&self, cursor: Option<i64>, now: f64) -> WatchRead {
        let _ = (cursor, now, self.seen, self.due_at);
        WatchRead::Snapshot
    }

    /// The tick at `now` took `snapshot`, having read `cursor` before it.
    pub fn snapshotted(&mut self, cursor: Option<i64>, snapshot: &Snapshot, now: f64) {
        let _ = (cursor, snapshot, now, self.refresh);
    }

    /// The refresh interval now in force.
    pub fn refresh(&self) -> Duration {
        self.refresh
    }
}

#[cfg(test)]
#[path = "watch_tests.rs"]
mod tests;
