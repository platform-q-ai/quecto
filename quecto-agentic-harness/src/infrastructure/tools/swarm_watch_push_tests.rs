//! #2390: the run watch is push-driven. Between ticks it waits on the
//! board's latch until nudged (a control change made in this process, a
//! pushed `watch` or a `wake`), or until its schedule is due (the safety
//! refresh, or a running run's deadline). An idle member so reads the
//! board a handful of times where it read it twice a second, and a pushed
//! change is still observed at once. Real board, real `SwarmContext`.
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use serde_json::json;

use super::{Tick, WatchObserver, watch_tick, watch_until_terminal};
use crate::application::swarm::dto::DroppedRecords;
use crate::application::swarm::ports::{
    BoardOpLog, CoordinationPort, SessionOpLog, SwarmRunControl,
};
use crate::composition::swarm::{board_wire, build_swarm_board_handles};
use crate::domain::swarm::watch::{Nudge, REFRESH_MAX, REFRESH_MIN, WatchSchedule};
use crate::domain::swarm::{BoardOpObservation, RunStatus, Snapshot, SwarmRunSummary};
use crate::infrastructure::tools::swarm_bridge::{
    Participation, SwarmBoard, SwarmContext, WatchNudges,
};

/// The `_watch` calls a board made, as the event log counts them.
#[derive(Default)]
struct Recorded(Mutex<Vec<BoardOpObservation>>);

impl BoardOpLog for Recorded {
    fn record(&self, observation: BoardOpObservation) {
        self.0.lock().unwrap().push(observation);
    }

    fn summarize(&self, _summary: SwarmRunSummary) {}
}

impl SessionOpLog for Recorded {
    fn dropped(&self, _drops: DroppedRecords) {}

    fn take_unnoted(&self) -> DroppedRecords {
        DroppedRecords::default()
    }
}

impl Recorded {
    /// The `_watch` calls recorded: an aggregate's `polls`, else one.
    fn watches(&self) -> u64 {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|record| record.op == "_watch")
            .map(|record| record.detail.polls.unwrap_or(1))
            .sum()
    }
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

fn context_over(checkout: &std::path::Path, board: SwarmBoard) -> SwarmContext {
    SwarmContext {
        lifecycle: Arc::new(crate::application::swarm::LifecycleService),
        checkout: checkout.to_path_buf(),
        member: "coordinator".into(),
        board,
    }
}

/// A run, running until `deadline`, created in a fresh checkout. The
/// watching harness's context records its board calls; `other` is another
/// harness's, on a board of its own (another process).
struct Run {
    _directory: tempfile::TempDir,
    watching: SwarmContext,
    log: Arc<Recorded>,
    other: SwarmContext,
}

fn running_until(deadline: f64) -> Run {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join(".quecto")).unwrap();
    let board = SwarmBoard::new(build_swarm_board_handles, board_wire());
    let log = Arc::new(Recorded::default());
    assert!(board.record_in(log.clone()));
    let watching = context_over(directory.path(), board);
    let other = context_over(
        directory.path(),
        SwarmBoard::new(build_swarm_board_handles, board_wire()),
    );
    // Created by the other harness, so no nudge of its own is latched on
    // the watching harness's board.
    other
        .call(
            "create",
            json!(["watch", [], [{"id":"tests","kind":"command","description":"pass"}], 3, deadline]),
        )
        .unwrap();
    Run {
        _directory: directory,
        watching,
        log,
        other,
    }
}

fn running() -> Run {
    running_until(now() + 3_600.0)
}

/// What the watch did, and when.
#[derive(Debug)]
enum Seen {
    Paused(Instant, Snapshot),
    Announced(Snapshot),
    Ended(Snapshot),
}

struct Observer(mpsc::Sender<Seen>);

impl WatchObserver for Observer {
    fn paused(&mut self, snapshot: &Snapshot) -> bool {
        let _ = self.0.send(Seen::Paused(Instant::now(), snapshot.clone()));
        true
    }

    fn announce(&mut self, snapshot: &Snapshot) {
        let _ = self.0.send(Seen::Announced(snapshot.clone()));
    }
}

/// Watches the run on a thread of its own, as `supervise` does.
fn watch(run: &Run) -> mpsc::Receiver<Seen> {
    let (seen, events) = mpsc::channel();
    let context = run.watching.clone();
    let snapshot = context.snapshot().unwrap();
    std::thread::spawn(move || {
        let mut observer = Observer(seen.clone());
        let ended =
            watch_until_terminal(&context, snapshot, &Participation::shared(), &mut observer);
        context.flush_watch_polls();
        let _ = seen.send(Seen::Ended(ended));
    });
    events
}

/// The next pause the watch observed, within `within`.
fn next_pause(events: &mpsc::Receiver<Seen>, within: Duration) -> (Instant, Snapshot) {
    let until = Instant::now() + within;
    loop {
        let left = until.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(Seen::Paused(at, snapshot)) => return (at, snapshot),
            Ok(Seen::Announced(_)) => {}
            Ok(Seen::Ended(snapshot)) => panic!("the watch ended first: {snapshot:?}"),
            Err(error) => panic!("no pause observed within {within:?}: {error}"),
        }
    }
}

/// Cancels the run from the other harness and pushes the nudge its
/// `watch` would deliver; answers what the watch announced meanwhile and
/// the snapshot it ended on.
fn cancel(run: &Run, events: &mpsc::Receiver<Seen>) -> (Vec<Snapshot>, Snapshot) {
    run.other.cancel_run().unwrap();
    run.watching.nudge_watch();
    let mut announced = Vec::new();
    loop {
        match events.recv_timeout(Duration::from_secs(3)) {
            Ok(Seen::Announced(snapshot)) => announced.push(snapshot),
            Ok(Seen::Paused(..)) => {}
            Ok(Seen::Ended(snapshot)) => return (announced, snapshot),
            Err(error) => panic!("the pushed cancellation was not observed: {error}"),
        }
    }
}

/// The latch: a nudge before the wait is not lost, a nudge ends a wait
/// early, a nudge is taken once, and an unnudged wait times out.
#[test]
fn a_nudge_ends_the_wait_early_and_is_never_lost() {
    let nudges = Arc::new(WatchNudges::default());
    nudges.nudge(Nudge::Remote);
    let started = Instant::now();
    assert_eq!(nudges.wait(Duration::from_secs(10)), Some(Nudge::Remote));
    assert!(started.elapsed() < Duration::from_secs(1), "not lost");
    assert_eq!(nudges.wait(Duration::from_millis(20)), None, "taken once");
    let waiting = nudges.clone();
    let waiter = std::thread::spawn(move || {
        let started = Instant::now();
        (waiting.wait(Duration::from_secs(10)), started.elapsed())
    });
    std::thread::sleep(Duration::from_millis(100));
    nudges.nudge(Nudge::Local);
    let (nudge, waited) = waiter.join().unwrap();
    assert_eq!(nudge, Some(Nudge::Local));
    assert!(waited < Duration::from_secs(2), "{waited:?}");
}

#[test]
fn a_local_nudge_outranks_a_remote_one_latched_with_it() {
    let nudges = WatchNudges::default();
    nudges.nudge(Nudge::Local);
    nudges.nudge(Nudge::Remote);
    assert_eq!(nudges.wait(Duration::from_secs(5)), Some(Nudge::Local));
    assert_eq!(nudges.wait(Duration::ZERO), None);
}

/// (a) A board op in this process that changed the run's control state
/// nudges its watch; a read, and an op that found the run as asked, do
/// not.
#[test]
fn a_control_change_on_this_board_nudges_its_watch() {
    let run = running();
    let creator = run.other.board.watch_nudges();
    assert_eq!(creator.wait(Duration::ZERO), Some(Nudge::Local), "create");
    let nudges = run.watching.board.watch_nudges();
    assert_eq!(
        nudges.wait(Duration::ZERO),
        None,
        "nothing changed here yet"
    );
    run.watching.summary().unwrap();
    assert_eq!(nudges.wait(Duration::ZERO), None, "a read");
    run.watching.pause("operator").unwrap();
    assert_eq!(nudges.wait(Duration::ZERO), Some(Nudge::Local), "a pause");
    run.watching.pause("operator").unwrap();
    assert_eq!(nudges.wait(Duration::ZERO), None, "already paused");
    run.other.resume_external().unwrap();
    assert_eq!(nudges.wait(Duration::ZERO), None, "another board's change");
}

/// Acceptance: with nothing changing, a supervised member makes a handful
/// of `_watch` calls over several seconds, where the fixed tick made two a
/// second; a pushed cancellation still ends the watch at once.
#[test]
fn an_idle_watch_reads_the_board_a_handful_of_times_not_twice_a_second() {
    let run = running();
    let events = watch(&run);
    std::thread::sleep(Duration::from_secs(3));
    let started = Instant::now();
    let (announced, ended) = cancel(&run, &events);
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(ended.status, RunStatus::Cancelled);
    assert!(announced.is_empty(), "another harness cancelled it");
    let watches = run.log.watches();
    assert!(watches <= 3, "{watches} board reads in 3 idle seconds");
}

/// A pause made by another process is observed as soon as its pushed
/// nudge arrives, well inside the refresh, without polling for it.
#[test]
fn a_pause_pushed_from_another_process_is_observed_within_two_seconds() {
    let run = running();
    let events = watch(&run);
    std::thread::sleep(Duration::from_millis(1_500));
    run.other.pause("operator").unwrap();
    let pushed = Instant::now();
    run.watching.nudge_watch();
    let (at, snapshot) = next_pause(&events, REFRESH_MIN);
    assert!(at - pushed < Duration::from_secs(2), "{:?}", at - pushed);
    assert_eq!(snapshot.status, RunStatus::Paused);
    let (_, ended) = cancel(&run, &events);
    assert_eq!(ended.status, RunStatus::Cancelled);
    let watches = run.log.watches();
    assert!(
        watches <= 3,
        "{watches} board reads: the pause was pushed, not polled"
    );
}

/// A lost push: with no nudge, the watch still observes the pause by its
/// safety refresh.
#[test]
fn a_lost_push_is_still_observed_by_the_refresh() {
    let run = running();
    let started = Instant::now();
    let events = watch(&run);
    std::thread::sleep(Duration::from_millis(300));
    run.other.pause("operator").unwrap();
    let (at, snapshot) = next_pause(&events, REFRESH_MIN + Duration::from_secs(2));
    assert!(at - started <= REFRESH_MIN + Duration::from_secs(1));
    assert_eq!(snapshot.status, RunStatus::Paused);
    // Paused and idle, it reads nothing more until nudged.
    std::thread::sleep(Duration::from_millis(1_500));
    let (_, ended) = cancel(&run, &events);
    assert_eq!(ended.status, RunStatus::Cancelled);
    let watches = run.log.watches();
    assert!(watches <= 3, "{watches} board reads: the refresh found it");
}

/// A running run's deadline, which no event announces, is observed at the
/// deadline itself without a nudge.
#[test]
fn a_running_runs_deadline_is_observed_at_the_deadline_without_a_nudge() {
    let deadline = now() + 2.5;
    let run = running_until(deadline);
    let events = watch(&run);
    let (_, snapshot) = next_pause(&events, Duration::from_secs(4));
    let observed = now();
    assert!(observed >= deadline, "{observed} before {deadline}");
    assert!(observed < deadline + 1.0, "{} late", observed - deadline);
    assert_eq!(snapshot.outcome, Some(RunStatus::BudgetExhausted));
    let (_, ended) = cancel(&run, &events);
    assert_eq!(ended.status, RunStatus::Cancelled);
    let watches = run.log.watches();
    assert!(
        watches <= 3,
        "{watches} board reads before and at the deadline"
    );
}

/// (a) and (b): a pause made in this process nudges its own watch, which
/// observes it at once and announces it to the other members, once; a
/// change another harness pushed is never announced again.
#[test]
fn a_control_change_made_in_this_process_is_observed_and_announced_once() {
    let run = running();
    let events = watch(&run);
    std::thread::sleep(Duration::from_millis(300));
    run.watching.pause("operator").unwrap();
    let paused = Instant::now();
    let (at, snapshot) = next_pause(&events, REFRESH_MIN);
    assert!(at - paused < Duration::from_secs(1), "{:?}", at - paused);
    assert_eq!(snapshot.status, RunStatus::Paused);
    run.other.resume_external().unwrap();
    run.watching.nudge_watch();
    std::thread::sleep(Duration::from_millis(300));
    let (announced, ended) = cancel(&run, &events);
    assert_eq!(ended.status, RunStatus::Cancelled);
    let statuses: Vec<_> = announced.iter().map(|snapshot| snapshot.status).collect();
    assert_eq!(
        statuses,
        [RunStatus::Paused],
        "only this process's own change"
    );
}

/// Owner decision (#2390): the refresh backs off to ten minutes. A long
/// refresh never leaves the watch acting on a stale view: every change the
/// watch acts on writes an event, so the next tick, however far its
/// refresh is, answers the run as it is now.
#[test]
fn a_long_refresh_never_leaves_the_watch_acting_on_a_stale_view() {
    let run = running();
    let mut snapshot = run.watching.snapshot().unwrap();
    let mut schedule = WatchSchedule::new();
    let participation = Participation::shared();
    let mut at = now();
    loop {
        let tick = watch_tick(
            &run.watching,
            &mut schedule,
            &mut snapshot,
            &participation,
            at,
        );
        assert_eq!(tick, Tick::Snapshot, "each due tick refreshes");
        if schedule.refresh() == REFRESH_MAX {
            break;
        }
        at += schedule.refresh().as_secs_f64();
    }
    run.other.call("_admit", json!(["worker", "r"])).unwrap();
    run.other.pause("operator").unwrap();
    let tick = watch_tick(
        &run.watching,
        &mut schedule,
        &mut snapshot,
        &participation,
        at + 1.0,
    );
    assert_eq!(
        tick,
        Tick::Snapshot,
        "the moved cursor answers the snapshot"
    );
    assert_eq!(snapshot.status, RunStatus::Paused);
    assert!(
        snapshot.members.iter().any(|member| member.id == "worker"),
        "{snapshot:?}"
    );
}
