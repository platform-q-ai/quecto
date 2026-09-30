//! #2338: every watch tick passes the board the cursor of its last
//! snapshot, so the board answers `unchanged` until the cursor moves; the
//! schedule passes none, asking for the snapshot whatever the cursor, only
//! when the running run's deadline came or a bounded refresh is due.
use super::*;
use crate::domain::swarm::{OWNER_IDLE_AFTER, RunStatus, Snapshot};

const NOW: f64 = 1_000_000.0;
const TICK: f64 = 0.5;

fn run(status: RunStatus, deadline: f64) -> Snapshot {
    Snapshot {
        control_generation: 0,
        status,
        outcome: None,
        coordinator: "coordinator".into(),
        deadline,
        members: Vec::new(),
    }
}

/// A run far from its deadline.
fn running() -> Snapshot {
    run(RunStatus::Running, NOW + 86_400.0)
}

/// A schedule that took its first snapshot at `NOW`, at `cursor`.
fn watched(cursor: i64, snapshot: &Snapshot) -> WatchSchedule {
    let mut schedule = WatchSchedule::new();
    assert_eq!(schedule.since(NOW), None, "the first tick asks for it");
    schedule.snapshotted(cursor, snapshot, NOW);
    schedule
}

#[test]
fn the_first_tick_asks_for_the_snapshot() {
    assert_eq!(WatchSchedule::new().since(NOW), None);
}

#[test]
fn a_tick_passes_the_last_snapshots_cursor_until_a_refresh_is_due() {
    let schedule = watched(7, &running());
    assert_eq!(schedule.since(NOW + TICK), Some(7));
    assert_eq!(
        schedule.since(NOW + REFRESH_MIN.as_secs_f64() - TICK),
        Some(7)
    );
    assert_eq!(
        schedule.since(NOW + REFRESH_MIN.as_secs_f64()),
        None,
        "the refresh is due"
    );
}

/// Wake latency: the board answers the snapshot as soon as its cursor is
/// not the one passed, so a change is seen on the very next tick, which
/// comes as often as the snapshot poll it replaces.
#[test]
fn the_watch_ticks_as_often_as_the_poll_it_replaces() {
    assert_eq!(WATCH_TICK, std::time::Duration::from_millis(500));
}

#[test]
fn an_unreadable_tick_asks_for_the_snapshot_next() {
    let mut schedule = watched(7, &running());
    schedule.unreadable();
    assert_eq!(schedule.since(NOW + TICK), None);
}

/// A running run's deadline is announced by no event: the snapshot (whose
/// gate records the expiry) is due at the deadline itself.
#[test]
fn a_running_runs_deadline_is_due_at_the_deadline() {
    let deadline = NOW + 2.0;
    let schedule = watched(7, &run(RunStatus::Running, deadline));
    assert_eq!(schedule.since(deadline - TICK), Some(7));
    assert_eq!(schedule.since(deadline), None);
    assert_eq!(schedule.since(deadline + TICK), None);
}

#[test]
fn a_passed_deadline_keeps_the_snapshot_due_while_the_run_reads_running() {
    let schedule = watched(7, &run(RunStatus::Running, NOW - 1.0));
    assert_eq!(schedule.since(NOW + TICK), None);
}

#[test]
fn only_a_running_runs_deadline_is_scheduled() {
    for status in [RunStatus::Setup, RunStatus::Paused] {
        let schedule = watched(7, &run(status, NOW + 1.0));
        assert_eq!(
            schedule.since(NOW + 2.0),
            Some(7),
            "{status:?}: its deadline expires nothing"
        );
    }
}

#[test]
fn the_refresh_backs_off_while_nothing_changes_and_restarts_after_a_change() {
    let mut schedule = watched(7, &running());
    let mut at = NOW;
    let mut intervals = Vec::new();
    for _ in 0..7 {
        let next = at + schedule.refresh().as_secs_f64();
        assert_eq!(schedule.since(next - TICK), Some(7));
        assert_eq!(schedule.since(next), None);
        schedule.snapshotted(7, &running(), next);
        intervals.push((next - at) as u64);
        at = next;
    }
    assert_eq!(intervals, [5, 10, 20, 40, 60, 60, 60]);
    schedule.snapshotted(8, &running(), at + TICK);
    assert_eq!(schedule.refresh(), REFRESH_MIN, "a change restarts it");
    assert_eq!(schedule.since(at + 1.0), Some(8));
}

/// The idle watch over an hour at its tick: no gap between snapshots is
/// longer than [`REFRESH_MAX`], and after the first minute it asks for at
/// least ten times fewer snapshots than the poll it replaces (one a tick).
#[test]
fn an_idle_hour_asks_for_ten_times_fewer_snapshots_within_the_refresh_bound() {
    let mut schedule = WatchSchedule::new();
    let ticks = 3_600 * 2;
    let mut taken = Vec::new();
    for tick in 0..ticks {
        let now = NOW + f64::from(tick) * TICK;
        // An idle board answers `unchanged` to its own cursor.
        if schedule.since(now) != Some(7) {
            schedule.snapshotted(7, &running(), now);
            taken.push(now);
        }
    }
    let longest = taken
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .fold(0.0, f64::max);
    assert!(longest <= REFRESH_MAX.as_secs_f64(), "{longest}");
    let after_first_minute = taken.iter().filter(|at| **at >= NOW + 60.0).count();
    let per_minute = after_first_minute as f64 / 59.0;
    assert!(
        per_minute * 10.0 <= 120.0,
        "{per_minute} snapshots a minute against the 120 of one per tick"
    );
}

/// Liveness: the longest the watch holds a view without a snapshot is well
/// inside the time an owner takes to turn idle (and so inside the idle
/// threshold plus a lost harness's grace).
#[test]
fn the_watchs_longest_stale_view_is_inside_owner_idle() {
    let stale = REFRESH_MAX.as_secs_f64() + WATCH_TICK.as_secs_f64();
    assert!(stale < OWNER_IDLE_AFTER, "{stale}");
    assert!(REFRESH_MIN <= REFRESH_MAX);
}
