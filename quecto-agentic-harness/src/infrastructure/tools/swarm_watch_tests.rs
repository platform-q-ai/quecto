//! #2338: the run watch reads the cheap event cursor every tick and takes
//! a `_snapshot` only when its schedule says so; its unchanged polls are
//! aggregated in the event log and still counted, and its cursor reads
//! never wait for, or hold off, a writer's lock.
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use serde_json::json;

use super::watch_tick;
use crate::application::swarm::dto::DroppedRecords;
use crate::application::swarm::ports::CoordinationPort;
use crate::application::swarm::ports::{BoardOpLog, SessionOpLog};
use crate::application::swarm::watch::WatchSchedule;
use crate::composition::swarm::{board_wire, build_swarm_board_handles};
use crate::domain::swarm::{BoardOpObservation, RunStatus, SwarmRunSummary};
use crate::infrastructure::tools::swarm_bridge::{Participation, SwarmBoard, SwarmContext};

/// The event log, in memory: records and summaries.
#[derive(Default)]
struct Recorded {
    ops: Mutex<Vec<BoardOpObservation>>,
    summaries: Mutex<Vec<SwarmRunSummary>>,
}

impl BoardOpLog for Recorded {
    fn record(&self, observation: BoardOpObservation) {
        self.ops.lock().unwrap().push(observation);
    }

    fn summarize(&self, summary: SwarmRunSummary) {
        self.summaries.lock().unwrap().push(summary);
    }
}

impl SessionOpLog for Recorded {
    fn dropped(&self, _drops: DroppedRecords) {}

    fn take_unnoted(&self) -> DroppedRecords {
        DroppedRecords::default()
    }
}

impl Recorded {
    fn taken(&self) -> Vec<BoardOpObservation> {
        std::mem::take(&mut *self.ops.lock().unwrap())
    }
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// A running run in a fresh checkout, created by `coordinator` on a board
/// recording in the returned log.
fn running() -> (tempfile::TempDir, SwarmContext, Arc<Recorded>) {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join(".quecto")).unwrap();
    let board = SwarmBoard::new(build_swarm_board_handles, board_wire());
    let log = Arc::new(Recorded::default());
    assert!(board.record_in(log.clone()));
    let context = SwarmContext {
        lifecycle: Arc::new(crate::application::swarm::LifecycleService),
        checkout: directory.path().to_path_buf(),
        member: "coordinator".into(),
        board,
    };
    let deadline = now() + 3_600.0;
    context
        .call(
            "create",
            json!(["watch", [], [{"id":"tests","kind":"command","description":"pass"}], 3, deadline]),
        )
        .unwrap();
    (directory, context, log)
}

/// The polls `records` account for: an aggregate's `polls`, else one.
fn polls(records: &[BoardOpObservation]) -> u64 {
    records
        .iter()
        .filter(|record| record.op == "_event_cursor")
        .map(|record| record.detail.polls.unwrap_or(1))
        .sum()
}

fn snapshots(records: &[BoardOpObservation]) -> usize {
    records
        .iter()
        .filter(|record| record.op == "_snapshot")
        .count()
}

/// Ticks the watch `ticks` times, half a second apart from `start` on the
/// board's clock; returns the ticks that took a snapshot.
fn watch(
    context: &SwarmContext,
    schedule: &mut WatchSchedule,
    snapshot: &mut crate::domain::swarm::Snapshot,
    start: f64,
    ticks: u32,
) -> usize {
    let participation = Participation::shared();
    (0..ticks)
        .filter(|tick| {
            let at = start + f64::from(*tick) * 0.5;
            watch_tick(context, schedule, snapshot, &participation, at)
        })
        .count()
}

/// An idle minute of the watch: at least ten times fewer `_snapshot`s
/// than the one a tick it replaces, every cursor poll still accounted for
/// in far fewer records, and none of them slowed by the busy handler.
#[test]
fn an_idle_minute_takes_ten_times_fewer_snapshots_and_counts_every_poll() {
    let (_directory, context, log) = running();
    let mut snapshot = context.snapshot().unwrap();
    let _setup = log.taken();
    let mut schedule = WatchSchedule::new();
    let taken = watch(&context, &mut schedule, &mut snapshot, now(), 120);
    context.flush_watch_polls();
    let records = log.taken();
    assert!(taken <= 12, "{taken} snapshots in 120 ticks");
    assert_eq!(snapshots(&records), taken, "each snapshot is recorded");
    assert_eq!(polls(&records), 120, "every poll is accounted for");
    assert!(
        records.len() <= 12 + 4,
        "{} records for an idle minute",
        records.len()
    );
    assert!(
        records.iter().all(|record| record.busy != Some(true)),
        "{records:?}"
    );
    assert!(
        records
            .iter()
            .all(|record| record.role == Some(crate::domain::swarm::BoardRole::Host)),
        "the watch's calls are the harness's own"
    );
}

/// Wake latency: a pause lands between two ticks, and the next tick takes
/// the snapshot that sees it.
#[test]
fn a_pause_is_seen_on_the_next_tick() {
    let (_directory, context, _log) = running();
    let mut snapshot = context.snapshot().unwrap();
    let mut schedule = WatchSchedule::new();
    let start = now();
    let _warm = watch(&context, &mut schedule, &mut snapshot, start, 4);
    context.pause("operator").unwrap();
    let participation = Participation::shared();
    assert!(watch_tick(
        &context,
        &mut schedule,
        &mut snapshot,
        &participation,
        start + 2.5
    ));
    assert_eq!(snapshot.status, RunStatus::Paused);
    assert!(participation.participating());
}

/// The run's summary counts the polls the watch still held when it was
/// written: the fold's `_event_cursor` count equals the polls made.
#[test]
fn the_run_summary_counts_the_polls_still_held() {
    let (_directory, context, log) = running();
    let mut snapshot = context.snapshot().unwrap();
    let mut schedule = WatchSchedule::new();
    let _ticks = watch(&context, &mut schedule, &mut snapshot, now(), 20);
    let settled = crate::domain::swarm::Snapshot {
        status: RunStatus::Cancelled,
        ..snapshot.clone()
    };
    assert!(context.summarize_settled(&settled));
    let summaries = log.summaries.lock().unwrap().clone();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].ops["_event_cursor"].ok, 20, "{summaries:?}");
    let written = log.ops.lock().unwrap().clone();
    assert_eq!(polls(&written), 20, "the held polls were written first");
}

/// A second connection holding `BEGIN IMMEDIATE` on the board until told.
fn holding_the_write_lock(
    database: std::path::PathBuf,
) -> (mpsc::Sender<()>, std::thread::JoinHandle<()>) {
    let (held, taken) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let thread = std::thread::spawn(move || {
        let connection = rusqlite::Connection::open(database).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        held.send(()).unwrap();
        let _told = released.recv_timeout(Duration::from_secs(30));
        connection.execute_batch("COMMIT").unwrap();
    });
    taken.recv_timeout(Duration::from_secs(30)).unwrap();
    (release, thread)
}

/// The watch's cursor read takes no write lock: a writer holding one
/// (between `BEGIN IMMEDIATE` and its commit) neither refuses it nor
/// makes it wait, and it is recorded as not busy.
#[test]
fn a_cursor_poll_does_not_wait_for_a_writers_lock() {
    let (_directory, context, log) = running();
    let _setup = log.taken();
    let (release, holder) = holding_the_write_lock(context.database());
    let started = Instant::now();
    let cursor = context.watch_cursor();
    let waited = started.elapsed();
    drop(release);
    holder.join().unwrap();
    assert!(cursor.unwrap() > 0, "the run's events");
    assert!(waited < Duration::from_millis(250), "{waited:?}");
    let records = log.taken();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0].busy, Some(false), "{records:?}");
}
