//! The run watch's tick (#2338): one `_watch` call, passing the cursor the
//! domain's [`WatchSchedule`] names, which answers the run's snapshot only
//! when the board changed or the schedule asked for it.
use super::super::swarm_bridge::{Participation, RunWatch, SwarmContext};
use crate::application::swarm::ports::Clock;
use std::time::{Duration, Instant};

use crate::domain::swarm::watch::{Announcement, Nudge, REMOTE_FLOOR, WatchSchedule};
use crate::domain::swarm::{RunStatus, Snapshot};

/// What one tick of the watch read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tick {
    /// The run's snapshot: the board changed, or the schedule asked for it.
    Snapshot,
    /// Only the cursor, which had not moved.
    Unchanged,
    /// Nothing: the call failed. The snapshot held is kept (as the poll
    /// kept it), and the next tick asks for the snapshot.
    Unreadable,
}

/// One tick of the watch at `now` (Unix seconds, the board's clock):
/// refreshes `snapshot` when the board answered one, and participation
/// from the snapshot held.
pub(super) fn watch_tick(
    context: &SwarmContext,
    schedule: &mut WatchSchedule,
    snapshot: &mut Snapshot,
    participation: &Participation,
    now: f64,
) -> Tick {
    let tick = match context.watch(schedule.since(now)) {
        Ok(RunWatch {
            event_cursor,
            snapshot: Some(current),
        }) => {
            *snapshot = current;
            schedule.snapshotted(event_cursor, snapshot, now);
            Tick::Snapshot
        }
        Ok(RunWatch { snapshot: None, .. }) => Tick::Unchanged,
        Err(error) => {
            tracing::error!(%error, "swarm supervisor lost coordination; retaining ownership");
            schedule.unreadable();
            Tick::Unreadable
        }
    };
    participation.set(crate::domain::swarm::participates(snapshot.deadline));
    tick
}

/// What the watch does about what it observes: the lifecycle's (the
/// supervisor's), or a test's.
pub(super) trait WatchObserver {
    /// The run is paused at a control generation not yet suspended:
    /// suspend this process's inference; whether it did.
    fn paused(&mut self, snapshot: &Snapshot) -> bool;
    /// This process changed the run's control state: push a `watch` to
    /// every other live member (#2390).
    fn announce(&mut self, snapshot: &Snapshot);
}

/// How long an ended watch, once settled, waits for this process's ops in
/// flight that may end the run (review M1): an op whose end the watch read
/// has committed, and only its event-log record and its return remain
/// before it latches its nudge; this bounds a stalled record.
const LATE_LOCAL: std::time::Duration = std::time::Duration::from_secs(2);

/// What a watch that ended answers: the snapshot it ended on (its status
/// the observed outcome), and what it owes the other members.
pub(super) struct Watched {
    pub(super) snapshot: Snapshot,
    announcement: Announcement,
}

/// After settling (review round 2, L2): pushes a change this process made
/// whose local nudge came late.
pub(super) fn announce_late(
    context: &SwarmContext,
    watched: &mut Watched,
    observer: &mut dyn WatchObserver,
) {
    // Review M1: an op of this process whose end the watch read may not
    // have latched its nudge yet; once it has, the end is still pushed.
    let nudges = context.board.watch_nudges(&context.database());
    if nudges.take_late_local(LATE_LOCAL)
        && watched
            .announcement
            .observed(Some(Nudge::Local), Some(&watched.snapshot))
    {
        observer.announce(&watched.snapshot);
    }
}

/// Watches the run from `snapshot` until its outcome is terminal, and
/// answers the snapshot it ended on (its status the observed outcome).
/// Between ticks it waits on the board's latch until nudged, or until its
/// schedule is due (#2390).
pub(super) fn watch_until_terminal(
    context: &SwarmContext,
    mut snapshot: Snapshot,
    participation: &Participation,
    observer: &mut dyn WatchObserver,
) -> Watched {
    let clock = super::SystemClock;
    let nudges = context.board.watch_nudges(&context.database());
    let mut suspended = None;
    let mut schedule = WatchSchedule::new();
    let mut announcement = Announcement::new(&snapshot);
    // The first tick is the watch's own, nudged by nobody.
    let mut cause = None;
    loop {
        let ticked = Instant::now();
        let tick = watch_tick(
            context,
            &mut schedule,
            &mut snapshot,
            participation,
            clock.now_seconds(),
        );
        // This process's own inference is suspended first, so an
        // unreachable member never delays it (review L3); then the change
        // this process made is pushed to the others.
        if snapshot.status == RunStatus::Paused && suspended != Some(snapshot.control_generation) {
            if observer.paused(&snapshot) {
                suspended = Some(snapshot.control_generation);
            }
        } else if snapshot.status == RunStatus::Running {
            suspended = None;
        }
        let read = match tick {
            Tick::Snapshot | Tick::Unchanged => Some(&snapshot),
            Tick::Unreadable => None,
        };
        if announcement.observed(cause, read) {
            observer.announce(&snapshot);
        }
        let outcome = context.lifecycle.observed_outcome(&snapshot, &clock);
        if outcome.terminal() {
            snapshot.status = outcome;
            debug_assert!(
                snapshot.status.terminal(),
                "the watch ends on a terminal run"
            );
            return Watched {
                snapshot,
                announcement,
            };
        }
        cause = nudges.wait(schedule.wait(clock.now_seconds()));
        // Review round 2, L1: a tick a remote nudge woke comes at most every
        // REMOTE_FLOOR; the pushes meanwhile are read by that one tick, and a
        // local nudge is never held.
        cause = match cause {
            Some(Nudge::Remote) => match REMOTE_FLOOR.saturating_sub(ticked.elapsed()) {
                Duration::ZERO => cause,
                left => nudges
                    .wait_floor(left)
                    .map_or(cause, |more| Some(more.merge(Nudge::Remote))),
            },
            Some(Nudge::Local) | None => cause,
        };
    }
}

#[cfg(test)]
#[path = "swarm_watch_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "swarm_watch_push_tests.rs"]
mod push_tests;
