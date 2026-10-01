//! The run watch's schedule (#2338, #2390): when a member's harness,
//! watching its run, reads the board, and when it asks it for a full
//! snapshot.
//!
//! The watch is push-driven (#2390): it has no fixed tick. Between ticks it
//! waits until nudged, or until its schedule is due ([`WatchSchedule::wait`]).
//! A nudge ([`Nudge`]) comes from a board op in this process that changed
//! the run's control state ([`changes_run_control`]), from another member's
//! harness that made such a change and pushed a no-turn `watch`, or from a
//! `wake` hint. The harness that made a change pushes it to every other
//! live member once ([`Announcement`]). The nudge is latched, so one that
//! arrives before the wait starts is never lost.
//!
//! Each tick is one board call, `_watch`, in one read transaction that
//! takes no write lock: given the event cursor the watch last saw
//! (`since`), the board answers `unchanged` when the cursor is still there,
//! and the run's snapshot with its cursor otherwise. The schedule passes no
//! cursor, so the board answers the snapshot whatever the cursor, only
//! when:
//!
//! - the last tick could not be read, or no snapshot was taken yet (such a
//!   tick is due at once, and the watch retries after [`RETRY`]);
//! - the run is running and its deadline has come: a passed deadline is
//!   the one change no event announces (the board records it on the next
//!   gated op, and `_watch` is one), so the snapshot is due at the deadline
//!   itself, and the unnudged watch wakes then;
//! - a refresh is due: a safety net for a lost push, and for a change made
//!   without an event (only a board edited from outside makes one). The
//!   refresh backs off while nothing changes, from [`REFRESH_MIN`] doubling
//!   to [`REFRESH_MAX`], and restarts at [`REFRESH_MIN`] after a change.
//!
//! Every change the watch acts on (a pause or resume, a run's end, a
//! deadline extension, a run created over the container's placeholder)
//! writes an event in the same transaction, so it moves the cursor, and the
//! next tick's answer is the snapshot, however long since the last one.
//!
//! Owner liveness is not the watch's to detect: a task owner turns idle
//! by the store's clock when any member reads it (`summary`, `task`,
//! `tasks`, [`OWNER_IDLE_AFTER`](crate::domain::swarm::OWNER_IDLE_AFTER)),
//! and a lost harness is recorded by `reconcile` or the launcher's reaper
//! after its grace. The watch reads neither, and it acts (suspends or
//! settles) only on a snapshot read on the tick that observed the change,
//! so its ten-minute refresh ceiling (owner decision, #2390), past the idle
//! threshold, holds no stale view it would act on.
use std::time::Duration;

use super::{RunStatus, Snapshot};

/// How long a tick that is due at once waits (#2390): one that could not
/// be read, or a passed deadline the board still reads as running. The
/// poll the watch replaced ran at this, so the watch never spins on the
/// board.
pub const RETRY: Duration = Duration::from_millis(500);

/// The refresh interval after a change: the first safety snapshot while
/// nothing moves comes this long after the last.
pub const REFRESH_MIN: Duration = Duration::from_secs(5);

/// The longest the watch goes without a snapshot while nothing changes:
/// ten minutes (owner decision, #2390), the safety net for a lost push.
pub const REFRESH_MAX: Duration = Duration::from_secs(600);

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
    /// its next tick (#2390): until its snapshot is due, at the refresh or
    /// at a running run's deadline. A tick already due (the last could not
    /// be read, or a passed deadline still reads running) waits [`RETRY`].
    pub fn wait(&self, now: f64) -> Duration {
        let ahead = self.due_at - now;
        match self.seen {
            Some(_) if ahead > 0.0 => {
                // A clock that stepped back is never waited out past the
                // ceiling.
                Duration::try_from_secs_f64(ahead.min(REFRESH_MAX.as_secs_f64())).unwrap_or(RETRY)
            }
            Some(_) | None => RETRY,
        }
    }
}

/// What woke the watch before its schedule was due (#2390).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nudge {
    /// Another member's harness pushed a `watch` (it changed the run's
    /// control state, and told every member itself) or a `wake`.
    Remote,
    /// A board op in this process changed the run's control state: only
    /// this process tells the other members.
    Local,
}

impl Nudge {
    /// Two nudges latched before the watch took them: a local one wins,
    /// as only it obliges the watch to tell the others.
    pub fn merge(self, other: Self) -> Self {
        match (self, other) {
            (Self::Remote, Self::Remote) => Self::Remote,
            (Self::Local, _) | (_, Self::Local) => Self::Local,
        }
    }
}

/// The run's control state, as the watch acts on it: what a pause, a
/// resume, an end or a deadline extension changes.
#[derive(Clone, Debug, PartialEq)]
struct ControlView {
    status: RunStatus,
    outcome: Option<RunStatus>,
    generation: u64,
    deadline: f64,
}

impl ControlView {
    fn of(snapshot: &Snapshot) -> Self {
        Self {
            status: snapshot.status,
            outcome: snapshot.outcome,
            generation: snapshot.control_generation,
            deadline: snapshot.deadline,
        }
    }
}

/// Whether this process owes the other members a `watch` push (#2390): it
/// does when a tick after one of its own (local) nudges reads a control
/// state the members were not told of. A change another member pushed is
/// never pushed again, and one a scheduled tick saw first is still pushed
/// on the local nudge's tick.
#[derive(Clone, Debug)]
pub struct Announcement {
    /// The control state the members were last told of, or found out.
    told: ControlView,
    /// A local nudge whose tick has not yet read the board.
    owed: bool,
}

impl Announcement {
    /// The obligation of a watch that starts from `snapshot`, which every
    /// member reads for itself.
    pub fn new(snapshot: &Snapshot) -> Self {
        Self {
            told: ControlView::of(snapshot),
            owed: false,
        }
    }

    /// A tick woken by `cause` (`None`: the schedule) read the run as
    /// `snapshot` (`None`: it could not be read, and a local nudge is
    /// carried to the next readable tick). Whether to push a `watch` to
    /// every other member now.
    pub fn observed(&mut self, cause: Option<Nudge>, snapshot: Option<&Snapshot>) -> bool {
        self.owed |= cause == Some(Nudge::Local);
        let Some(snapshot) = snapshot else {
            return false;
        };
        let view = ControlView::of(snapshot);
        let changed = view != self.told;
        let announce = self.owed && changed;
        // A readable tick after a local nudge settles it: the change is
        // told now, or the op changed nothing.
        self.owed = false;
        match (announce, cause) {
            // Told now, or by the member that pushed it to every member.
            (true, _) | (false, Some(Nudge::Remote)) => self.told = view,
            (false, Some(Nudge::Local) | None) => {}
        }
        announce
    }
}

/// Whether board op `op`, answering with `decision`, changed the run's
/// control state (#2390): an affirmative list of the ops and the decisions
/// that changed the status, the control generation or the deadline. An op
/// that found the run as asked (`unchanged`), a read, and every other op
/// nudge nobody. A deadline the gate of any op records as passed is not
/// listed: every watch is due at the deadline itself.
pub fn changes_run_control(op: &str, decision: &str) -> bool {
    matches!(
        (op, decision),
        ("create", "fresh" | "over_setup")
            | ("pause", "paused")
            | ("resume" | "_resume_external", "resumed")
            | ("_close", "closed")
            | ("_extend_deadline", "extended")
            | ("stop", "stopped")
            | ("complete", "completed")
            | (
                "usage_budget" | "_record_request" | "_request_admission",
                "paused"
            )
            | ("_quarantine", "recorded")
            | ("_confirmed_dead", "confirmed" | "coordinator_confirmed")
            | ("_lose_coordinator", "lost")
    )
}

#[cfg(test)]
#[path = "watch_tests.rs"]
mod tests;
