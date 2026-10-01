//! #2338: every watch tick passes the board the cursor of its last
//! snapshot, so the board answers `unchanged` until the cursor moves; the
//! schedule passes none, asking for the snapshot whatever the cursor, only
//! when the running run's deadline came or a bounded refresh is due.
use super::*;
use std::time::Duration;

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

/// #2390: no fixed tick. Unnudged, the watch waits until its snapshot is
/// due (the refresh, or a running run's deadline when that is sooner), not
/// half a second; a nudge ends the wait early (the latch's own tests).
#[test]
fn the_wait_is_until_the_snapshot_is_due_not_a_fixed_tick() {
    let schedule = watched(7, &running());
    assert_eq!(schedule.wait(NOW), REFRESH_MIN);
    assert_eq!(
        schedule.wait(NOW + 1.5),
        REFRESH_MIN - Duration::from_millis(1_500)
    );
    let deadline = NOW + 2.0;
    let schedule = watched(7, &run(RunStatus::Running, deadline));
    assert_eq!(
        schedule.wait(NOW + 0.5),
        Duration::from_millis(1_500),
        "a running run's deadline is sooner than the refresh"
    );
}

/// A tick that is due at once (the last could not be read, or a passed
/// deadline still reads running) waits the retry the poll ran at, so the
/// watch never spins on the board.
#[test]
fn a_tick_already_due_waits_the_retry_and_never_spins() {
    assert_eq!(WatchSchedule::new().wait(NOW), RETRY);
    let mut schedule = watched(7, &running());
    schedule.unreadable();
    assert_eq!(schedule.wait(NOW + TICK), RETRY);
    let schedule = watched(7, &run(RunStatus::Running, NOW - 1.0));
    assert_eq!(schedule.wait(NOW + TICK), RETRY);
    assert_eq!(watched(7, &running()).wait(NOW + 5.0), RETRY);
    assert_eq!(RETRY, Duration::from_millis(500));
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
    for _ in 0..9 {
        let next = at + schedule.refresh().as_secs_f64();
        assert_eq!(schedule.since(next - TICK), Some(7));
        assert_eq!(schedule.since(next), None);
        schedule.snapshotted(7, &running(), next);
        intervals.push((next - at) as u64);
        at = next;
    }
    // Owner decision (#2390): the ceiling is ten minutes.
    assert_eq!(intervals, [5, 10, 20, 40, 80, 160, 320, 600, 600]);
    schedule.snapshotted(8, &running(), at + TICK);
    assert_eq!(schedule.refresh(), REFRESH_MIN, "a change restarts it");
    assert_eq!(schedule.since(at + 1.0), Some(8));
}

/// The idle watch over an hour, waking only when its schedule is due
/// (#2390): about one board read a minute in the first ten minutes and one
/// every ten minutes after, where the fixed tick read twice a second; no
/// gap between snapshots is longer than [`REFRESH_MAX`].
#[test]
fn an_idle_hour_reads_the_board_about_once_a_minute_not_twice_a_second() {
    let mut schedule = WatchSchedule::new();
    let mut now = NOW;
    let mut taken = Vec::new();
    while now < NOW + 3_600.0 {
        // An idle board answers `unchanged` to its own cursor; every tick
        // the unnudged watch makes is a due one, so each takes the snapshot.
        assert_eq!(schedule.since(now), None, "an unnudged tick is a due one");
        schedule.snapshotted(7, &running(), now);
        taken.push(now);
        now += schedule.wait(now).as_secs_f64();
    }
    let longest = taken
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .fold(0.0, f64::max);
    assert!(longest <= REFRESH_MAX.as_secs_f64(), "{longest}");
    assert!(
        taken.len() <= 15,
        "{} board reads in an idle hour",
        taken.len()
    );
}

/// Owner decision (#2390): the safety refresh backs off to ten minutes,
/// past the time an owner takes to turn idle. That no longer matters to
/// the watch: it never decides on owner liveness (the store's reads do, by
/// its own clock) and acts only on a snapshot it read on the tick that saw
/// the change (pinned by `a_long_refresh_never_leaves_the_watch_acting_on_a_stale_view`
/// in the harness's watch tests); a running run's deadline still bounds
/// every wait.
#[test]
fn the_refresh_ceiling_is_ten_minutes_by_owner_decision() {
    assert_eq!(REFRESH_MIN, Duration::from_secs(5));
    assert_eq!(REFRESH_MAX, Duration::from_secs(600));
    assert!(REFRESH_MAX.as_secs_f64() > OWNER_IDLE_AFTER);
    let mut schedule = watched(7, &running());
    let mut at = NOW;
    while schedule.refresh() < REFRESH_MAX {
        at += schedule.refresh().as_secs_f64();
        schedule.snapshotted(7, &running(), at);
    }
    let deadline = at + 30.0;
    schedule.snapshotted(7, &run(RunStatus::Running, deadline), at);
    assert_eq!(schedule.refresh(), REFRESH_MAX);
    assert_eq!(
        schedule.wait(at),
        Duration::from_secs(30),
        "the deadline bounds the wait at the ceiling"
    );
}

/// A local nudge outranks a remote one: only it obliges the watch to tell
/// the others.
#[test]
fn a_local_nudge_outranks_a_remote_one() {
    assert_eq!(Nudge::Remote.merge(Nudge::Local), Nudge::Local);
    assert_eq!(Nudge::Local.merge(Nudge::Remote), Nudge::Local);
    assert_eq!(Nudge::Remote.merge(Nudge::Remote), Nudge::Remote);
    assert_eq!(Nudge::Local.merge(Nudge::Local), Nudge::Local);
}

fn paused(generation: u64) -> Snapshot {
    Snapshot {
        control_generation: generation,
        ..run(RunStatus::Paused, NOW + 86_400.0)
    }
}

/// The member whose process changed the run's control state tells the
/// others, once. Only an announcement settles what the members were told:
/// a change read on a remote nudge (or a due tick) may be this process's
/// own, read before its local nudge was latched (review M1), so the local
/// nudge's tick still announces it. A rare duplicate push is the price.
#[test]
fn a_local_change_is_announced_once_even_when_a_remote_tick_read_it_first() {
    let mut announcement = Announcement::new(&running());
    assert!(!announcement.observed(None, Some(&running())));
    assert!(
        !announcement.observed(Some(Nudge::Local), Some(&running())),
        "no change"
    );
    assert!(
        !announcement.observed(Some(Nudge::Remote), Some(&paused(3))),
        "a remote nudge obliges nothing"
    );
    assert!(
        announcement.observed(Some(Nudge::Local), Some(&paused(3))),
        "this process's own pause, read first on the remote tick"
    );
    assert!(
        !announcement.observed(Some(Nudge::Local), Some(&paused(3))),
        "told already"
    );
    assert!(!announcement.observed(Some(Nudge::Remote), Some(&running())));
    assert!(!announcement.observed(None, Some(&running())));
}

/// A change this process made but a scheduled tick saw first is still
/// announced on the local nudge's tick, which reads it unchanged; a local
/// nudge on an unreadable tick is carried to the next readable one.
#[test]
fn a_local_change_is_announced_however_the_ticks_fall() {
    let mut announcement = Announcement::new(&running());
    assert!(
        !announcement.observed(None, Some(&paused(3))),
        "a due tick tells nobody"
    );
    assert!(announcement.observed(Some(Nudge::Local), Some(&paused(3))));
    let mut announcement = Announcement::new(&running());
    assert!(!announcement.observed(Some(Nudge::Local), None));
    assert!(announcement.observed(None, Some(&paused(3))));
    let deadline = Snapshot {
        deadline: NOW + 1.0,
        ..running()
    };
    let mut announcement = Announcement::new(&running());
    assert!(
        announcement.observed(Some(Nudge::Local), Some(&deadline)),
        "an extension is a control change"
    );
}
