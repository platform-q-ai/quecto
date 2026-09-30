//! #2338: the watch reads the event cursor every tick and takes a snapshot
//! only when the cursor moved, the running run's deadline came, or a
//! bounded refresh is due; a change is still seen within one tick.
use super::*;
use crate::application::swarm::board_loss::LOSS_GRACE;
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

/// A schedule that took its first snapshot at `NOW`, having read `cursor`.
fn watched(cursor: i64, snapshot: &Snapshot) -> WatchSchedule {
    let mut schedule = WatchSchedule::new();
    assert_eq!(schedule.read(Some(cursor), NOW), WatchRead::Snapshot);
    schedule.snapshotted(Some(cursor), snapshot, NOW);
    schedule
}

#[test]
fn the_first_tick_takes_a_snapshot() {
    assert_eq!(WatchSchedule::new().read(Some(0), NOW), WatchRead::Snapshot);
    assert_eq!(WatchSchedule::new().read(None, NOW), WatchRead::Snapshot);
}

#[test]
fn an_unchanged_cursor_takes_no_snapshot_until_a_refresh_is_due() {
    let schedule = watched(7, &running());
    assert_eq!(schedule.read(Some(7), NOW + TICK), WatchRead::Skip);
    assert_eq!(
        schedule.read(Some(7), NOW + REFRESH_MIN.as_secs_f64() - TICK),
        WatchRead::Skip
    );
    assert_eq!(
        schedule.read(Some(7), NOW + REFRESH_MIN.as_secs_f64()),
        WatchRead::Snapshot,
        "the refresh is due"
    );
}

/// Wake latency: a board change (a pause, a resume, the run's end) moves
/// the cursor, and the very next tick takes the snapshot that sees it.
#[test]
fn a_moved_cursor_takes_a_snapshot_on_the_next_tick() {
    let schedule = watched(7, &running());
    assert_eq!(schedule.read(Some(8), NOW + TICK), WatchRead::Snapshot);
    assert_eq!(
        WATCH_TICK,
        std::time::Duration::from_millis(500),
        "the tick the snapshot poll ran at, so a change is seen as soon as before"
    );
}

#[test]
fn an_unreadable_cursor_takes_a_snapshot() {
    let schedule = watched(7, &running());
    assert_eq!(schedule.read(None, NOW + TICK), WatchRead::Snapshot);
}

/// A running run's deadline is announced by no event: the snapshot (whose
/// gate records the expiry) is due at the deadline itself.
#[test]
fn a_running_runs_deadline_is_due_at_the_deadline() {
    let deadline = NOW + 2.0;
    let schedule = watched(7, &run(RunStatus::Running, deadline));
    assert_eq!(schedule.read(Some(7), deadline - TICK), WatchRead::Skip);
    assert_eq!(schedule.read(Some(7), deadline), WatchRead::Snapshot);
    assert_eq!(schedule.read(Some(7), deadline + TICK), WatchRead::Snapshot);
}

#[test]
fn a_passed_deadline_keeps_the_snapshot_due_while_the_run_reads_running() {
    let deadline = NOW - 1.0;
    let schedule = watched(7, &run(RunStatus::Running, deadline));
    assert_eq!(schedule.read(Some(7), NOW + TICK), WatchRead::Snapshot);
}

#[test]
fn only_a_running_runs_deadline_is_scheduled() {
    for status in [RunStatus::Setup, RunStatus::Paused] {
        let schedule = watched(7, &run(status, NOW + 1.0));
        assert_eq!(
            schedule.read(Some(7), NOW + 2.0),
            WatchRead::Skip,
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
        assert_eq!(schedule.read(Some(7), next - TICK), WatchRead::Skip);
        assert_eq!(schedule.read(Some(7), next), WatchRead::Snapshot);
        schedule.snapshotted(Some(7), &running(), next);
        intervals.push((next - at) as u64);
        at = next;
    }
    assert_eq!(intervals, [5, 10, 20, 40, 60, 60, 60]);
    schedule.snapshotted(Some(8), &running(), at + TICK);
    assert_eq!(schedule.refresh(), REFRESH_MIN, "a change restarts it");
}

/// The idle watch over an hour at its tick: no gap between snapshots is
/// longer than [`REFRESH_MAX`], and after the first minute it takes at
/// least ten times fewer snapshots than the poll it replaces (one a tick).
#[test]
fn an_idle_hour_takes_ten_times_fewer_snapshots_within_the_refresh_bound() {
    let mut schedule = WatchSchedule::new();
    let ticks = 3_600 * 2;
    let mut taken = Vec::new();
    for tick in 0..ticks {
        let now = NOW + f64::from(tick) * TICK;
        if schedule.read(Some(7), now) == WatchRead::Snapshot {
            schedule.snapshotted(Some(7), &running(), now);
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
/// inside the time an owner takes to turn idle, and a lost harness's grace.
#[test]
fn the_watchs_longest_stale_view_is_inside_owner_idle_and_loss_grace() {
    let stale = REFRESH_MAX.as_secs_f64() + WATCH_TICK.as_secs_f64();
    assert!(stale < OWNER_IDLE_AFTER + LOSS_GRACE, "{stale}");
    assert!(REFRESH_MIN <= REFRESH_MAX);
}
