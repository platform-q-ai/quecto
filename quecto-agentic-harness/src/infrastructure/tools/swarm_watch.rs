//! The run watch's tick (#2338): one `_watch` call, passing the cursor the
//! domain's [`WatchSchedule`] names, which answers the run's snapshot only
//! when the board changed or the schedule asked for it.
use super::super::swarm_bridge::{Participation, RunWatch, SwarmContext};
use crate::domain::swarm::Snapshot;
use crate::domain::swarm::watch::WatchSchedule;

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

#[cfg(test)]
#[path = "swarm_watch_tests.rs"]
mod tests;
