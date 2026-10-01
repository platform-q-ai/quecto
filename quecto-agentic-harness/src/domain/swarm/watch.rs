//! The run watch's schedule (#2338): when a member's harness, watching its
//! run, asks the board for a full snapshot.
//!
//! The watch ticks every [`WATCH_TICK`], as it always has, so a board
//! change is seen as soon as before. Each tick is one board call, `_watch`,
//! in one read transaction that takes no write lock: given the event cursor
//! the watch last saw (`since`), the board answers `unchanged` when the
//! cursor is still there, and the run's snapshot with its cursor otherwise.
//! The schedule passes no cursor, so the board answers the snapshot
//! whatever the cursor, only when:
//!
//! - the last tick could not be read, or no snapshot was taken yet;
//! - the run is running and its deadline has come: a passed deadline is
//!   the one change no event announces (the board records it on the next
//!   gated op, and `_watch` is one), so the snapshot is due at the deadline
//!   itself, not one refresh later;
//! - a refresh is due: a safety net for a change made without an event
//!   (only a board edited from outside makes one). The refresh backs off
//!   while nothing changes, from [`REFRESH_MIN`] doubling to
//!   [`REFRESH_MAX`], and restarts at [`REFRESH_MIN`] after a change.
//!
//! Every change the watch acts on (a pause or resume, a run's end, a
//! deadline extension, a run created over the container's placeholder)
//! writes an event in the same transaction, so it moves the cursor, and the
//! next tick's answer is the snapshot.
//!
//! Owner liveness is not the watch's to detect: a task owner turns idle
//! by the store's clock when any member reads it (`summary`, `task`,
//! `tasks`, [`OWNER_IDLE_AFTER`](crate::domain::swarm::OWNER_IDLE_AFTER)),
//! and a lost harness is recorded by `reconcile` after its grace. The
//! watch's longest gap between snapshots stays well inside the idle
//! threshold, so it never holds a stale view past either.
use std::time::Duration;

use super::{RunStatus, Snapshot};

/// How often the watch reads the event cursor: the 500 ms the snapshot
/// poll it replaces ran at, so a change is observed within one tick.
pub const WATCH_TICK: Duration = Duration::from_millis(500);

/// How long a tick that is due at once waits (#2390).
pub const RETRY: Duration = WATCH_TICK;

/// The refresh interval after a change: the first safety snapshot while
/// nothing moves comes this long after the last.
pub const REFRESH_MIN: Duration = Duration::from_secs(5);

/// The longest the watch goes without a snapshot while nothing changes.
pub const REFRESH_MAX: Duration = Duration::from_secs(60);

/// When the watch next asks for a snapshot, from the last one it took.
#[derive(Clone, Debug)]
pub struct WatchSchedule {
    /// The cursor the last snapshot was taken at; `None` before the first,
    /// or after a tick that could not be read.
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
    /// A schedule that asks for a snapshot on its first tick.
    pub fn new() -> Self {
        Self {
            seen: None,
            due_at: f64::NEG_INFINITY,
            refresh: REFRESH_MIN,
        }
    }

    /// The cursor the tick at `now` passes the board: the one the last
    /// snapshot was taken at, so the board answers `unchanged` while it
    /// holds; `None` when a snapshot is due whatever the cursor.
    pub fn since(&self, now: f64) -> Option<i64> {
        match now >= self.due_at {
            true => None,
            false => self.seen,
        }
    }

    /// The tick at `now` could not be read: the next one asks for the
    /// snapshot.
    pub fn unreadable(&mut self) {
        self.seen = None;
    }

    /// The tick at `now` took `snapshot`, at event cursor `cursor`: the
    /// next snapshot is due one refresh later (sooner at a running run's
    /// deadline). The refresh doubles, up to [`REFRESH_MAX`], when the
    /// cursor had not moved since the last snapshot (a refresh of an
    /// unchanged board), and restarts at [`REFRESH_MIN`] when it had.
    pub fn snapshotted(&mut self, cursor: i64, snapshot: &Snapshot, now: f64) {
        debug_assert!(cursor >= 0, "an event cursor is never negative");
        let unchanged = self.seen == Some(cursor);
        self.refresh = match unchanged {
            true => (self.refresh * 2).min(REFRESH_MAX),
            false => REFRESH_MIN,
        };
        self.seen = Some(cursor);
        let refresh_at = now + self.refresh.as_secs_f64();
        // Only a running run's deadline ends anything (`observed_outcome`);
        // a passed one keeps the snapshot due every tick until the run
        // reads as paused, as the poll this replaces did.
        self.due_at = match snapshot.status {
            RunStatus::Running => refresh_at.min(snapshot.deadline),
            RunStatus::Setup
            | RunStatus::Paused
            | RunStatus::Succeeded
            | RunStatus::Blocked
            | RunStatus::Failed
            | RunStatus::Cancelled
            | RunStatus::BudgetExhausted => refresh_at,
        };
        debug_assert!(
            (REFRESH_MIN..=REFRESH_MAX).contains(&self.refresh),
            "the refresh stays within its bounds"
        );
        debug_assert!(
            self.due_at <= refresh_at,
            "a snapshot is never due later than one refresh on"
        );
    }

    /// The refresh interval now in force.
    pub fn refresh(&self) -> Duration {
        self.refresh
    }
}

impl WatchSchedule {
    /// How long the watch waits at `now`, when nothing nudges it, before
    /// its next tick (#2390).
    pub fn wait(&self, _now: f64) -> Duration {
        WATCH_TICK
    }
}

/// What woke the watch before its schedule was due (#2390).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nudge {
    /// Another member's harness pushed a `watch` or a `wake`.
    Remote,
    /// A board op in this process changed the run's control state.
    Local,
}

impl Nudge {
    /// Two nudges latched before the watch took them.
    pub fn merge(self, other: Self) -> Self {
        let _ = other;
        self
    }
}

/// Whether this process owes the other members a `watch` push (#2390).
#[derive(Clone, Debug)]
pub struct Announcement;

impl Announcement {
    /// The obligation of a watch that starts from `snapshot`.
    pub fn new(_snapshot: &Snapshot) -> Self {
        Self
    }

    /// A tick woken by `cause` read `snapshot`: whether to push now.
    pub fn observed(&mut self, _cause: Option<Nudge>, _snapshot: Option<&Snapshot>) -> bool {
        false
    }
}

/// Whether board op `op`, deciding `decision`, changed the run's control
/// state (#2390).
pub fn changes_run_control(_op: &str, _decision: &str) -> bool {
    false
}

#[cfg(test)]
#[path = "watch_tests.rs"]
mod tests;
