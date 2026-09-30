//! #2338: the run watch makes one `_watch` call a tick, which answers the
//! run's snapshot only when the board changed or the schedule asked for
//! it; its `unchanged` ticks are aggregated in the event log and still
//! counted (also when the board goes or recording stops), and its reads
//! never wait for, or hold off, a writer's lock.
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use serde_json::json;

use super::{Tick, watch_tick};
use crate::application::swarm::dto::DroppedRecords;
use crate::application::swarm::ports::CoordinationPort;
use crate::application::swarm::ports::{BoardOpLog, SessionOpLog};
use crate::composition::swarm::{board_wire, build_swarm_board_handles};
use crate::domain::swarm::watch::WatchSchedule;
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

/// The ticks `records` account for: an aggregate's `polls`, else one.
fn ticks(records: &[BoardOpObservation]) -> u64 {
    records
        .iter()
        .filter(|record| record.op == "_watch")
        .map(|record| record.detail.polls.unwrap_or(1))
        .sum()
}

/// The records of ticks that answered the snapshot.
fn snapshots(records: &[BoardOpObservation]) -> usize {
    records
        .iter()
        .filter(|record| record.op == "_watch" && record.decision.as_deref() == Some("snapshot"))
        .count()
}

/// Ticks the watch `ticks` times, half a second apart from `start` on the
/// board's clock; returns what each tick read.
fn watch(
    context: &SwarmContext,
    schedule: &mut WatchSchedule,
    snapshot: &mut crate::domain::swarm::Snapshot,
    start: f64,
    ticks: u32,
) -> Vec<Tick> {
    let participation = Participation::shared();
    (0..ticks)
        .map(|tick| {
            let at = start + f64::from(tick) * 0.5;
            watch_tick(context, schedule, snapshot, &participation, at)
        })
        .collect()
}

/// An idle minute of the watch: one `_watch` call a tick, at least ten
/// times fewer snapshots than the one a tick it replaces, every tick
/// accounted for in far fewer records, none slowed by the busy handler.
#[test]
fn an_idle_minute_takes_ten_times_fewer_snapshots_and_counts_every_tick() {
    let (_directory, context, log) = running();
    let mut snapshot = context.snapshot().unwrap();
    let _setup = log.taken();
    let mut schedule = WatchSchedule::new();
    let read = watch(&context, &mut schedule, &mut snapshot, now(), 120);
    context.flush_watch_polls();
    let records = log.taken();
    let taken = read.iter().filter(|tick| **tick == Tick::Snapshot).count();
    assert!(taken <= 12, "{taken} snapshots in 120 ticks");
    assert_eq!(snapshots(&records), taken, "each snapshot is recorded");
    assert_eq!(ticks(&records), 120, "every tick is accounted for");
    assert!(
        records.iter().all(|record| record.op == "_watch"),
        "one call a tick, nothing else: {records:?}"
    );
    assert!(records.len() <= 2 * 12, "{} records", records.len());
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

/// Review round 1, finding 5: a tick after a board change is one call and
/// one record, the snapshot read with the cursor, not a cursor read and
/// then a snapshot.
#[test]
fn a_tick_after_a_change_is_one_call_and_one_record() {
    let (_directory, context, log) = running();
    let mut snapshot = context.snapshot().unwrap();
    let mut schedule = WatchSchedule::new();
    let start = now();
    let _warm = watch(&context, &mut schedule, &mut snapshot, start, 2);
    context.flush_watch_polls();
    for n in 0..3 {
        context
            .call("task_create", json!([format!("r{n}"), "t", ["tests pass"]]))
            .unwrap();
        let _setup = log.taken();
        let participation = Participation::shared();
        let tick = watch_tick(
            &context,
            &mut schedule,
            &mut snapshot,
            &participation,
            start + 1.0 + f64::from(n) * 0.5,
        );
        assert_eq!(tick, Tick::Snapshot);
        let records = log.taken();
        assert_eq!(records.len(), 1, "one call, one record: {records:?}");
        assert_eq!(records[0].op, "_watch");
        assert_eq!(records[0].decision.as_deref(), Some("snapshot"));
        assert_eq!(records[0].cursor_moved, Some(true));
    }
}

/// Wake latency: a pause lands between two ticks, and the next tick reads
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
    assert_eq!(
        watch_tick(
            &context,
            &mut schedule,
            &mut snapshot,
            &participation,
            start + 2.5
        ),
        Tick::Snapshot
    );
    assert_eq!(snapshot.status, RunStatus::Paused);
    assert!(participation.participating());
}

/// Review round 1, finding 6: a tick that could not be read says so, keeps
/// the snapshot held, and asks for the snapshot on the next tick.
#[test]
fn an_unreadable_tick_is_reported_and_keeps_the_snapshot() {
    let (directory, context, _log) = running();
    let mut snapshot = context.snapshot().unwrap();
    let mut schedule = WatchSchedule::new();
    let start = now();
    let _warm = watch(&context, &mut schedule, &mut snapshot, start, 2);
    std::fs::remove_file(context.database()).unwrap();
    let participation = Participation::shared();
    let tick = watch_tick(
        &context,
        &mut schedule,
        &mut snapshot,
        &participation,
        start + 1.0,
    );
    assert_eq!(tick, Tick::Unreadable);
    assert_eq!(snapshot.status, RunStatus::Running, "the snapshot held");
    assert_eq!(
        schedule.since(start + 1.5),
        None,
        "the next tick asks for it"
    );
    drop(directory);
}

/// The run's summary counts the ticks the watch still held when it was
/// written: the fold's `_watch` count equals the ticks made.
#[test]
fn the_run_summary_counts_the_ticks_still_held() {
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
    assert_eq!(summaries[0].ops["_watch"].ok, 20, "{summaries:?}");
    let written = log.ops.lock().unwrap().clone();
    assert_eq!(ticks(&written), 20, "the held ticks were written first");
}

/// Review round 1, finding 1: dropping the board (the last context holding
/// it) writes the ticks it held.
#[test]
fn dropping_the_board_writes_the_ticks_it_held() {
    let (_directory, context, log) = running();
    let mut snapshot = context.snapshot().unwrap();
    let mut schedule = WatchSchedule::new();
    let _ticks = watch(&context, &mut schedule, &mut snapshot, now(), 10);
    let _setup_and_first = log.taken();
    drop(context);
    let written = log.taken();
    assert!(!written.is_empty(), "the held ticks were written");
    assert!(
        written.iter().all(|record| record.detail.polls.is_some()),
        "{written:?}"
    );
}

/// Review round 1, finding 1: recording stops only after the ticks held
/// until then were written.
#[test]
fn stopping_recording_writes_the_ticks_held() {
    let (_directory, context, log) = running();
    let mut snapshot = context.snapshot().unwrap();
    let mut schedule = WatchSchedule::new();
    let read = watch(&context, &mut schedule, &mut snapshot, now(), 10);
    let unchanged = read.iter().filter(|tick| **tick == Tick::Unchanged).count();
    assert!(unchanged > 0);
    let before = ticks(&log.ops.lock().unwrap());
    context.board.stop_recording();
    let after = ticks(&log.ops.lock().unwrap());
    assert_eq!(after, 10, "every tick is written before recording stops");
    assert!(after > before, "some were still held");
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

/// A tick takes no write lock: a writer holding one (between `BEGIN
/// IMMEDIATE` and its commit) neither refuses it nor makes it wait, for
/// the cursor alone or for the snapshot, and it is recorded as not busy.
#[test]
fn a_tick_does_not_wait_for_a_writers_lock() {
    let (_directory, context, log) = running();
    let cursor = context.watch(None).unwrap().event_cursor;
    context.flush_watch_polls();
    let _setup = log.taken();
    let (release, holder) = holding_the_write_lock(context.database());
    let started = Instant::now();
    let unchanged = context.watch(Some(cursor));
    let snapshot = context.watch(None);
    let waited = started.elapsed();
    drop(release);
    holder.join().unwrap();
    assert!(unchanged.unwrap().snapshot.is_none());
    assert_eq!(
        snapshot.unwrap().snapshot.unwrap().status,
        RunStatus::Running
    );
    assert!(waited < Duration::from_millis(250), "{waited:?}");
    context.flush_watch_polls();
    let records = log.taken();
    assert_eq!(ticks(&records), 2, "{records:?}");
    assert!(
        records.iter().all(|record| record.busy == Some(false)),
        "{records:?}"
    );
}

/// A tick at the deadline still records the expiry, as the gate always
/// did: the run reads as paused, holding `budget-exhausted` (the deadline
/// is moved into the past by hand, as the clock would), whatever cursor
/// the tick passed.
#[test]
fn a_tick_at_the_deadline_still_records_the_expiry() {
    let (_directory, context, _log) = running();
    let cursor = context.watch(None).unwrap().event_cursor;
    rusqlite::Connection::open(context.database())
        .unwrap()
        .execute("UPDATE run SET deadline = ?1", [now() - 1.0])
        .unwrap();
    let watched = context.watch(Some(cursor)).unwrap();
    let snapshot = watched.snapshot.expect("the expiry moved the cursor");
    assert_eq!(snapshot.status, RunStatus::Paused, "{snapshot:?}");
    assert_eq!(snapshot.outcome, Some(RunStatus::BudgetExhausted));
    assert!(watched.event_cursor > cursor);
}

/// Review round 1, finding 2: a board an older writer created, lacking a
/// column only the full transaction adds (`run.outcome`, which the gate
/// reads), fails the read transaction as a store failure; the tick and
/// the snapshot still answer, through the IMMEDIATE path, which adds the
/// column back.
#[test]
fn an_older_board_without_an_added_column_still_answers() {
    let (_directory, context, _log) = running();
    let connection = rusqlite::Connection::open(context.database()).unwrap();
    let columns = |connection: &rusqlite::Connection| -> Vec<String> {
        let mut statement = connection.prepare("PRAGMA table_info(run)").unwrap();
        statement
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    let drop_outcome = |connection: &rusqlite::Connection| {
        connection
            .execute_batch("ALTER TABLE run DROP COLUMN outcome")
            .unwrap();
        assert!(!columns(connection).contains(&"outcome".to_owned()));
    };
    drop_outcome(&connection);
    let watched = context.watch(None).unwrap();
    assert_eq!(
        watched.snapshot.unwrap().status,
        RunStatus::Running,
        "the tick answers"
    );
    assert!(
        columns(&connection).contains(&"outcome".to_owned()),
        "the full transaction added the column back"
    );
    drop_outcome(&connection);
    assert_eq!(context.snapshot().unwrap().status, RunStatus::Running);
    assert!(columns(&connection).contains(&"outcome".to_owned()));
}

/// `_watch`'s `since` is a cursor the board answered, or null.
#[test]
fn a_watch_refuses_a_since_that_is_no_cursor() {
    let (_directory, context, _log) = running();
    for since in [json!("7"), json!(-1), json!(1.5)] {
        let refused = context.call("_watch", json!([since])).unwrap_err();
        assert!(
            refused
                .to_string()
                .contains("since must be an event cursor"),
            "{refused}"
        );
    }
    assert!(context.call("_watch", json!([null])).unwrap()["snapshot"].is_object());
}
