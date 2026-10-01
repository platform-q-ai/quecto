//! The run watch's tick (#2338): one `_watch` call, passing the cursor the
//! domain's [`WatchSchedule`] names, which answers the run's snapshot only
//! when the board changed or the schedule asked for it.
use super::super::swarm_bridge::{Participation, RunWatch, SwarmContext};
use crate::application::swarm::ports::Clock;
use crate::domain::swarm::watch::{Announcement, WatchSchedule};
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

/// Watches the run from `snapshot` until its outcome is terminal, and
/// answers the snapshot it ended on (its status the observed outcome).
/// Between ticks it waits on the board's latch until nudged, or until its
/// schedule is due (#2390).
pub(super) fn watch_until_terminal(
    context: &SwarmContext,
    mut snapshot: Snapshot,
    participation: &Participation,
    observer: &mut dyn WatchObserver,
) -> Snapshot {
    let clock = super::SystemClock;
    let nudges = context.board.watch_nudges();
    let mut suspended = None;
    let mut schedule = WatchSchedule::new();
    let mut announcement = Announcement::new(&snapshot);
    // The first tick is the watch's own, nudged by nobody.
    let mut cause = None;
    loop {
        let tick = watch_tick(
            context,
            &mut schedule,
            &mut snapshot,
            participation,
            clock.now_seconds(),
        );
        let read = match tick {
            Tick::Snapshot | Tick::Unchanged => Some(&snapshot),
            Tick::Unreadable => None,
        };
        if announcement.observed(cause, read) {
            observer.announce(&snapshot);
        }
        if snapshot.status == RunStatus::Paused && suspended != Some(snapshot.control_generation) {
            if observer.paused(&snapshot) {
                suspended = Some(snapshot.control_generation);
            }
        } else if snapshot.status == RunStatus::Running {
            suspended = None;
        }
        let outcome = context.lifecycle.observed_outcome(&snapshot, &clock);
        if outcome.terminal() {
            snapshot.status = outcome;
            return snapshot;
        }
        cause = nudges.wait(schedule.wait(clock.now_seconds()));
    }
}

#[cfg(test)]
#[path = "swarm_watch_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "swarm_watch_push_tests.rs"]
mod push_tests;
