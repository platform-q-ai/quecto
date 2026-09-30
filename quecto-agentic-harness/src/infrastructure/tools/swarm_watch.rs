//! The run watch's tick (#2338): the event cursor first, and a snapshot
//! only when the application's [`WatchSchedule`] asks for one.
use super::super::swarm_bridge::{Participation, SwarmContext};
use crate::domain::swarm::Snapshot;
use crate::domain::swarm::watch::{WatchRead, WatchSchedule};

/// One tick of the watch at `now` (Unix seconds, the board's clock): reads
/// the event cursor, and when the schedule asks for it a snapshot, which
/// refreshes `snapshot` and participation. Returns whether it took one. A
/// snapshot that could not be read keeps the one held (as the poll did),
/// and the schedule, not told of it, asks again on the next tick.
pub(super) fn watch_tick(
    context: &SwarmContext,
    schedule: &mut WatchSchedule,
    snapshot: &mut Snapshot,
    participation: &Participation,
    now: f64,
) -> bool {
    let cursor = context.watch_cursor().ok();
    match schedule.read(cursor, now) {
        WatchRead::Snapshot => {
            if super::observe(context, snapshot, participation) {
                schedule.snapshotted(cursor, snapshot, now);
            }
            true
        }
        WatchRead::Skip => false,
    }
}

#[cfg(test)]
#[path = "swarm_watch_tests.rs"]
mod tests;
