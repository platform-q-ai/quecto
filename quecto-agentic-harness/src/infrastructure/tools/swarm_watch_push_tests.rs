//! #2390: the run watch is push-driven. Between ticks it waits on the
//! board's latch until nudged (a control change made in this process, a
//! pushed `watch` or a `wake`), or until its schedule is due (the safety
//! refresh, or a running run's deadline). An idle member so reads the
//! board a handful of times where it read it twice a second, and a pushed
//! change is still observed at once. Real board, real `SwarmContext`.
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use serde_json::json;

use super::{Tick, WatchObserver, announce_late, watch_tick, watch_until_terminal};
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

/// The `_watch` calls a board made, as the event log counts them; and,
/// as a seam for an op still in flight after its commit, a record of one
/// op that is slowed, saying first that it committed.
#[derive(Default)]
struct Recorded(Mutex<Vec<BoardOpObservation>>, Mutex<Option<Slowed>>);

/// The op whose record is slowed, by how long, and who is told it
/// committed.
struct Slowed {
    op: &'static str,
    by: Duration,
    committed: mpsc::Sender<()>,
}

impl BoardOpLog for Recorded {
    fn record(&self, observation: BoardOpObservation) {
        let slowed = self
            .1
            .lock()
            .unwrap()
            .as_ref()
            .filter(|slowed| slowed.op == observation.op)
            .map(|slowed| (slowed.by, slowed.committed.clone()));
        if let Some((by, committed)) = slowed {
            let _ = committed.send(());
            std::thread::sleep(by);
        }
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
    /// Slows the record of `op` by `by`; answers when each such op has
    /// committed.
    fn slow(&self, op: &'static str, by: Duration) -> mpsc::Receiver<()> {
        let (committed, told) = mpsc::channel();
        *self.1.lock().unwrap() = Some(Slowed { op, by, committed });
        told
    }

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
    /// The watch returned, ready to settle.
    Terminal(Instant),
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
        let mut watched =
            watch_until_terminal(&context, snapshot, &Participation::shared(), &mut observer);
        let _ = seen.send(Seen::Terminal(Instant::now()));
        // As `supervise`: the settlement, then the late push.
        announce_late(&context, &mut watched, &mut observer);
        context.flush_watch_polls();
        let _ = seen.send(Seen::Ended(watched.snapshot));
    });
    events
}

/// The next pause the watch observed, within `within`.
fn next_pause(events: &mpsc::Receiver<Seen>, within: Duration) -> (Instant, Snapshot) {
    next_pause_announcing(events, within, &mut Vec::new())
}

/// [`next_pause`], keeping what the watch announced before it.
fn next_pause_announcing(
    events: &mpsc::Receiver<Seen>,
    within: Duration,
    announced: &mut Vec<Snapshot>,
) -> (Instant, Snapshot) {
    let until = Instant::now() + within;
    loop {
        let left = until.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(Seen::Paused(at, snapshot)) => return (at, snapshot),
            Ok(Seen::Announced(snapshot)) => announced.push(snapshot),
            Ok(Seen::Terminal(_)) => {}
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
            Ok(Seen::Paused(..) | Seen::Terminal(_)) => {}
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
/// nudges the watch of that run's board file; a read, and an op that found
/// the run as asked, do not, nor does another process's change.
#[test]
fn a_control_change_on_this_board_nudges_its_watch() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join(".quecto")).unwrap();
    let creator = context_over(
        directory.path(),
        SwarmBoard::new(build_swarm_board_handles, board_wire()),
    );
    let nudges = creator.board.watch_nudges(&creator.database());
    creator
        .call(
            "create",
            json!(["watch", [], [{"id":"tests","kind":"command","description":"pass"}], 3, now() + 3_600.0]),
        )
        .unwrap();
    assert_eq!(nudges.wait(Duration::ZERO), Some(Nudge::Local), "create");
    let other = context_over(
        directory.path(),
        SwarmBoard::new(build_swarm_board_handles, board_wire()),
    );
    creator.summary().unwrap();
    assert_eq!(nudges.wait(Duration::ZERO), None, "a read");
    creator.pause("operator").unwrap();
    assert_eq!(nudges.wait(Duration::ZERO), Some(Nudge::Local), "a pause");
    creator.pause("operator").unwrap();
    assert_eq!(nudges.wait(Duration::ZERO), None, "already paused");
    other.resume_external().unwrap();
    assert_eq!(nudges.wait(Duration::ZERO), None, "another board's change");
}

/// Review L2: the latch is keyed by the run's board file. A control change
/// this process makes on another run's board (a nested launcher recording
/// its launchee's loss, the host recording a lost coordinator) nudges no
/// watch of its own run, and a board file nobody here watches gets no
/// latch at all.
#[test]
fn a_control_change_on_another_runs_board_nudges_nothing_here() {
    let run = running();
    let nested = running();
    // One process board serving both files, as a nested launcher's does.
    let launcher = SwarmContext {
        checkout: nested.watching.checkout.clone(),
        ..run.watching.clone()
    };
    let own = run.watching.board.watch_nudges(&run.watching.database());
    launcher.pause("the nested run").unwrap();
    assert_eq!(own.wait(Duration::ZERO), None, "another run's change");
    let hosted = crate::infrastructure::tools::swarm_bridge::HostedStore::at(
        nested.watching.checkout.clone(),
        run.watching.board.clone(),
    );
    hosted.record_lost_coordinator("coordinator").unwrap();
    assert_eq!(own.wait(Duration::ZERO), None, "a hosted loss elsewhere");
    assert!(
        !run.watching.board.watches(&nested.watching.database()),
        "no latch for a file nobody here watches"
    );
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
    // Well inside the 5 s refresh: the push, not the schedule, ended it.
    assert!(started.elapsed() < Duration::from_secs(2));
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
    // The first snapshot, the deadline's (one more when the wall clock
    // wakes the watch a hair early and it retries), and the cancellation's.
    let watches = run.log.watches();
    assert!(
        watches <= 4,
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
    let mut announced = Vec::new();
    let (at, snapshot) = next_pause_announcing(&events, REFRESH_MIN, &mut announced);
    assert!(at - paused < Duration::from_secs(1), "{:?}", at - paused);
    assert_eq!(snapshot.status, RunStatus::Paused);
    run.other.resume_external().unwrap();
    run.watching.nudge_watch();
    std::thread::sleep(Duration::from_millis(300));
    let (later, ended) = cancel(&run, &events);
    announced.extend(later);
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

/// Review L3: the watch suspends this process's own inference before it
/// pushes the change to the others, so an unreachable member never delays
/// it.
#[test]
fn the_watch_suspends_before_it_announces() {
    let run = running();
    let events = watch(&run);
    std::thread::sleep(Duration::from_millis(300));
    run.watching.pause("operator").unwrap();
    let first = events.recv_timeout(REFRESH_MIN).unwrap();
    assert!(matches!(first, Seen::Paused(..)), "{first:?}");
    let second = events.recv_timeout(REFRESH_MIN).unwrap();
    assert!(matches!(second, Seen::Announced(..)), "{second:?}");
    let (_, ended) = cancel(&run, &events);
    assert_eq!(ended.status, RunStatus::Cancelled);
}

/// Review M1: this process's pause commits, a remote nudge wakes the watch,
/// which reads the pause before the op latched its local nudge; the late
/// local nudge still announces it.
#[test]
fn a_local_change_read_first_on_a_remote_tick_is_still_announced() {
    let run = running();
    let events = watch(&run);
    std::thread::sleep(Duration::from_millis(300));
    let latch = run.watching.board.watch_nudges(&run.watching.database());
    // The op commits; its nudge is not latched yet.
    run.other.pause("operator").unwrap();
    latch.nudge(Nudge::Remote);
    let (_, snapshot) = next_pause(&events, REFRESH_MIN);
    assert_eq!(snapshot.status, RunStatus::Paused);
    // The op's late local nudge.
    latch.nudge(Nudge::Local);
    match events.recv_timeout(REFRESH_MIN) {
        Ok(Seen::Announced(snapshot)) => assert_eq!(snapshot.status, RunStatus::Paused),
        other => panic!("the late local nudge was not announced: {other:?}"),
    }
    let (_, ended) = cancel(&run, &events);
    assert_eq!(ended.status, RunStatus::Cancelled);
}

/// Review M1, the terminal case: this process's cancellation is still in
/// flight when a remote nudge makes the watch read it; the watch, about to
/// end, waits for the op and announces its late local nudge.
#[test]
fn a_local_terminal_change_still_in_flight_is_announced_before_the_watch_ends() {
    let run = running();
    let events = watch(&run);
    std::thread::sleep(Duration::from_millis(300));
    let latch = run.watching.board.watch_nudges(&run.watching.database());
    let op = latch.control_op();
    run.other.cancel_run().unwrap();
    latch.nudge(Nudge::Remote);
    std::thread::sleep(Duration::from_millis(300));
    op.finish(true);
    let mut announced = Vec::new();
    let ended = loop {
        match events.recv_timeout(Duration::from_secs(3)) {
            Ok(Seen::Announced(snapshot)) => announced.push(snapshot.status),
            Ok(Seen::Paused(..) | Seen::Terminal(_)) => {}
            Ok(Seen::Ended(snapshot)) => break snapshot,
            Err(error) => panic!("the watch did not end: {error}"),
        }
    };
    assert_eq!(ended.status, RunStatus::Cancelled);
    assert_eq!(announced, [RunStatus::Cancelled]);
}

/// Review round 2, L1: a received push has a rate floor. A flood of remote
/// nudges (every 2 ms) is read at most about ten times a second.
#[test]
fn a_flood_of_remote_nudges_is_read_at_most_ten_times_a_second() {
    let run = running();
    let events = watch(&run);
    std::thread::sleep(Duration::from_millis(300));
    let latch = run.watching.board.watch_nudges(&run.watching.database());
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(1_300) {
        latch.nudge(Nudge::Remote);
        std::thread::sleep(Duration::from_millis(2));
    }
    let (_, ended) = cancel(&run, &events);
    assert_eq!(ended.status, RunStatus::Cancelled);
    let watches = run.log.watches();
    assert!(watches <= 18, "{watches} board reads for a 1.3 s flood");
}

/// Review round 2, L1: the floor takes remote nudges without returning,
/// and gives way at once to a local one.
#[test]
fn the_floor_holds_remote_nudges_and_gives_way_to_a_local_one() {
    let nudges = Arc::new(WatchNudges::default());
    nudges.nudge(Nudge::Remote);
    let started = Instant::now();
    assert_eq!(
        nudges.wait_floor(Duration::from_millis(200)),
        Some(Nudge::Remote)
    );
    assert!(started.elapsed() >= Duration::from_millis(150), "held");
    let local = nudges.clone();
    let nudger = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        local.nudge(Nudge::Local);
    });
    let started = Instant::now();
    assert_eq!(
        nudges.wait_floor(Duration::from_secs(5)),
        Some(Nudge::Local)
    );
    assert!(started.elapsed() < Duration::from_secs(1), "gave way");
    nudger.join().unwrap();
}

/// Review round 2, L2: an op of this process that cannot end the run (a
/// request's accounting, on every model request) still in flight never
/// delays the settlement of a run another harness ended.
#[test]
fn an_unrelated_op_in_flight_does_not_delay_the_settlement() {
    let run = running();
    let events = watch(&run);
    std::thread::sleep(Duration::from_millis(300));
    let committed = run.log.slow("_record_request", Duration::from_secs(2));
    let accounting = run.watching.clone();
    let record = json!({"request_id": "q", "instrumented_attempts": 0, "outcome": "rejected"});
    let in_flight = std::thread::spawn(move || accounting.call("_record_request", json!([record])));
    committed.recv_timeout(Duration::from_secs(5)).unwrap();
    run.other.cancel_run().unwrap();
    let cancelled = Instant::now();
    run.watching.nudge_watch();
    let settled = loop {
        match events.recv_timeout(Duration::from_secs(5)).unwrap() {
            Seen::Terminal(at) => break at,
            Seen::Paused(..) | Seen::Announced(_) => {}
            Seen::Ended(snapshot) => panic!("ended before it settled: {snapshot:?}"),
        }
    };
    assert!(
        settled - cancelled < Duration::from_secs(1),
        "{:?}",
        settled - cancelled
    );
    in_flight.join().unwrap().unwrap();
}

/// Review round 2, L3: the production in-flight count. This process's own
/// cancellation commits, its record (and so its local nudge) is slowed,
/// and a remote nudge makes the watch read the end first; the watch, about
/// to end, waits for the op through `SwarmBoard::call_as`'s count and
/// still pushes the end.
#[test]
fn a_real_cancellation_racing_a_remote_nudge_is_still_pushed() {
    let run = running();
    let events = watch(&run);
    std::thread::sleep(Duration::from_millis(300));
    let committed = run.log.slow("stop", Duration::from_millis(500));
    let cancelling = run.watching.clone();
    let cancel = std::thread::spawn(move || cancelling.cancel_run());
    committed.recv_timeout(Duration::from_secs(5)).unwrap();
    run.watching.nudge_watch();
    let mut announced = Vec::new();
    let ended = loop {
        match events.recv_timeout(Duration::from_secs(5)).unwrap() {
            Seen::Announced(snapshot) => announced.push(snapshot.status),
            Seen::Paused(..) | Seen::Terminal(_) => {}
            Seen::Ended(snapshot) => break snapshot,
        }
    };
    cancel.join().unwrap().unwrap();
    assert_eq!(ended.status, RunStatus::Cancelled);
    assert_eq!(announced, [RunStatus::Cancelled]);
}
