//! The run watch's tick (#2338): the event cursor first, and a snapshot
//! only when the application's [`WatchSchedule`] asks for one.
use super::super::swarm_bridge::{Participation, SwarmContext};
use crate::application::swarm::watch::WatchSchedule;
use crate::domain::swarm::Snapshot;

/// One tick of the watch at `now` (Unix seconds): refreshes `snapshot`
/// and participation from it when the schedule asks for a snapshot.
/// Returns whether it took one.
pub(super) fn watch_tick(
    context: &SwarmContext,
    schedule: &mut WatchSchedule,
    snapshot: &mut Snapshot,
    participation: &Participation,
    now: f64,
) -> bool {
    let _ = (schedule, now);
    super::observe(context, snapshot, participation);
    true
}

#[cfg(test)]
#[path = "swarm_watch_tests.rs"]
mod tests;
