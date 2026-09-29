//! #2303: each board call records exactly one `swarm_op` in the event log
//! (only when it is switched on), with its refusal kind, its real lock
//! wait, and no board text.
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

use super::{BOARD_OPS, BoardTelemetry, Method, SwarmBoardHandles, call};
use crate::application::swarm::dto::BoardLocation;
use crate::application::swarm::ports::{
    BoardCallMeter, BoardOpLog, BoardRepository, CallMeasure, Clock, IdSource,
};
use crate::composition::swarm::{board_op_log, build_swarm_board_handles_with, with_event_log};
use crate::domain::swarm::{
    BoardError, BoardOpObservation, BoardOpOutcome, BoardRole, RefusalKind,
};
use crate::infrastructure::persistence::audit_log::AuditLog;
use crate::infrastructure::persistence::swarm_board::repository::SqliteBoardRepository;

struct Fixed(f64);
impl Clock for Fixed {
    fn now_seconds(&self) -> f64 {
        self.0
    }
}

#[derive(Default)]
struct Counter(Mutex<u64>);
impl IdSource for Counter {
    fn hex32(&self) -> String {
        let mut next = self.0.lock().unwrap();
        *next += 1;
        format!("{:032x}", *next)
    }
}

/// The event log, in memory.
#[derive(Default)]
struct Recorded(Mutex<Vec<BoardOpObservation>>);

impl BoardOpLog for Recorded {
    fn record(&self, observation: BoardOpObservation) {
        self.0.lock().unwrap().push(observation);
    }
}

impl Recorded {
    fn ops(&self) -> Vec<BoardOpObservation> {
        self.0.lock().unwrap().clone()
    }

    fn clear(&self) {
        self.0.lock().unwrap().clear();
    }
}

/// A meter that measures nothing and counts how often it was asked.
#[derive(Default)]
struct Unmeasured(Mutex<u32>);

impl BoardCallMeter for Unmeasured {
    fn metered(&self, work: &mut dyn FnMut()) -> Option<CallMeasure> {
        *self.0.lock().unwrap() += 1;
        work();
        None
    }
}

fn location(dir: &tempfile::TempDir) -> BoardLocation {
    BoardLocation {
        database: dir.path().join("swarm.sqlite"),
        checkout: dir.path().to_path_buf(),
    }
}

fn plain(repository: Arc<dyn BoardRepository>) -> SwarmBoardHandles {
    build_swarm_board_handles_with(
        repository,
        Arc::new(Fixed(1_000.0)),
        Arc::new(Counter::default()),
    )
}

/// A board file under a fresh directory and its handles, recording in
/// `log`.
fn logged(log: &Arc<Recorded>) -> (tempfile::TempDir, SwarmBoardHandles) {
    let dir = tempfile::TempDir::new().unwrap();
    let repository = Arc::new(SqliteBoardRepository::new(&location(&dir)));
    let handles = with_event_log(plain(repository), log.clone());
    (dir, handles)
}

fn create_args() -> Value {
    json!({
        "goal": "ship",
        "constraints": ["none"],
        "criteria": [{"id": "t", "kind": "command", "description": "test"}],
        "member_limit": 3,
        "deadline": 4_600.0,
    })
}

fn only(log: &Recorded) -> BoardOpObservation {
    let ops = log.ops();
    assert_eq!(ops.len(), 1, "{ops:?}");
    ops.into_iter().next().unwrap()
}

/// Every op the dispatcher serves, and a name it does not, leaves exactly
/// one `swarm_op` naming it, whatever it answers.
#[test]
fn every_board_op_emits_exactly_one_swarm_op() {
    assert!(BOARD_OPS.contains(&"_status") && BOARD_OPS.contains(&"_snapshot"));
    for &op in BOARD_OPS.iter().chain(&["no_such_method"]) {
        for args in [json!([]), json!({})] {
            let log = Arc::new(Recorded::default());
            let (_dir, handles) = logged(&log);
            call(&handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
            log.clear();
            let _answer = call(&handles, "parent", op, args);
            let recorded = only(&log);
            let named = match op {
                "no_such_method" => "unknown",
                known => known,
            };
            assert_eq!(recorded.op, named);
        }
    }
}

/// Every `Method` the dispatcher has. The `match` is exhaustive, so a new
/// method does not compile until it is listed here too, and then
/// [`BOARD_OPS`] must name it for the test below to pass.
fn every_method() -> Vec<Method> {
    let all = vec![
        Method::Status,
        Method::Snapshot,
        Method::CreateRun,
        Method::BootstrapRun,
    ];
    for method in &all {
        match method {
            Method::Status | Method::Snapshot | Method::CreateRun | Method::BootstrapRun => {}
        }
    }
    all
}

#[test]
fn board_ops_names_every_method_once() {
    let names: Vec<&str> = every_method().into_iter().map(Method::name).collect();
    let mut listed = BOARD_OPS.to_vec();
    listed.sort_unstable();
    let mut expected = names.clone();
    expected.sort_unstable();
    assert_eq!(listed, expected, "BOARD_OPS lists every method, once");
    for name in names {
        assert!(Method::parse(name).is_some(), "{name} parses");
    }
}

fn refusal(log: &Recorded, answer: Result<Value, BoardError>) -> RefusalKind {
    let error = answer.expect_err("the board refuses");
    let recorded = only(log);
    log.clear();
    let BoardOpOutcome::Refused { kind } = recorded.outcome else {
        panic!("a refusal records its kind: {recorded:?}");
    };
    assert_eq!(kind, error.kind(), "the record carries the error's kind");
    assert_eq!(recorded.result_bytes, 0);
    kind
}

#[test]
fn a_refused_op_records_its_kind() {
    let log = Arc::new(Recorded::default());
    let (_dir, handles) = logged(&log);
    assert_eq!(
        refusal(&log, call(&handles, "parent", "_status", json!([]))),
        RefusalKind::StoreMissing
    );
    assert_eq!(
        refusal(&log, call(&handles, "parent", "no_such_method", json!([]))),
        RefusalKind::Calling
    );
    assert_eq!(
        refusal(&log, call(&handles, "parent", "create_run", json!([1]))),
        RefusalKind::Calling
    );
    let mut invalid = create_args();
    invalid["member_limit"] = json!(true);
    assert_eq!(
        refusal(&log, call(&handles, "parent", "create_run", invalid)),
        RefusalKind::Invalid
    );
    call(&handles, "parent", "create_run", create_args()).unwrap();
    log.clear();
    assert_eq!(
        refusal(&log, call(&handles, "parent", "create_run", create_args())),
        RefusalKind::RunExists
    );
    assert_eq!(
        refusal(&log, call(&handles, "ghost", "_snapshot", json!([]))),
        RefusalKind::NotMember
    );
}

#[test]
fn an_answered_op_records_its_run_role_size_and_measure() {
    let log = Arc::new(Recorded::default());
    let (_dir, handles) = logged(&log);
    call(&handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
    log.clear();
    let answer = call(&handles, "parent", "_status", json!([])).unwrap();
    let recorded = only(&log);
    assert_eq!(recorded.op, "_status");
    assert_eq!(recorded.outcome, BoardOpOutcome::Ok);
    assert_eq!(recorded.role, BoardRole::Host);
    assert_eq!(recorded.actor_ref.as_str(), "parent");
    assert_eq!(
        recorded.run_id.as_deref(),
        Some("00000000000000000000000000000001"),
        "the run the placeholder was created under"
    );
    assert_eq!(
        recorded.result_bytes,
        u64::try_from(serde_json::to_vec(&answer).unwrap().len()).unwrap()
    );
    assert_eq!(recorded.busy, Some(false), "{recorded:?}");
    assert_eq!(recorded.busy_wait_us, Some(0), "{recorded:?}");
    assert!(!recorded.cursor_moved);
    let lock_wait = recorded.lock_wait_us.expect("a transaction was measured");
    assert!(recorded.duration_us >= lock_wait, "{recorded:?}");
    assert_eq!((recorded.task_id, recorded.message_id), (None, None));
}

/// What was not measured is `null`, never a zero that reads as real
/// (#2303 review H2): a call that began no transaction, or a meter that
/// measured nothing.
#[test]
fn an_unmeasured_op_records_null_waits() {
    let log = Arc::new(Recorded::default());
    let (_dir, handles) = logged(&log);
    let _refused = call(&handles, "parent", "no_such_method", json!([]));
    let recorded = only(&log);
    assert_eq!(
        (
            recorded.lock_wait_us,
            recorded.busy_wait_us,
            recorded.busy,
            recorded.run_id
        ),
        (None, None, None, None),
        "no transaction began"
    );
    log.clear();
    let meter = Arc::new(Unmeasured::default());
    let dir = tempfile::TempDir::new().unwrap();
    let handles = SwarmBoardHandles {
        telemetry: Some(BoardTelemetry {
            log: log.clone(),
            meter: meter.clone(),
        }),
        ..plain(Arc::new(SqliteBoardRepository::new(&location(&dir))))
    };
    call(&handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
    let recorded = only(&log);
    assert_eq!(recorded.outcome, BoardOpOutcome::Ok);
    assert_eq!((recorded.lock_wait_us, recorded.busy), (None, None));
    assert_eq!(*meter.0.lock().unwrap(), 1, "one measure per call");
}

/// A second connection holding `BEGIN IMMEDIATE` on the board until told
/// to let go (#2303 review L5: no timer races the call). Returns once the
/// lock is held, and the sender that lets it go.
fn hold(dir: &tempfile::TempDir) -> (mpsc::Sender<()>, thread::JoinHandle<()>) {
    let path = location(dir).database;
    let (held, taken) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let holder = thread::spawn(move || {
        let connection = rusqlite::Connection::open(path).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        held.send(()).unwrap();
        let _told = released.recv_timeout(Duration::from_secs(30));
        connection.execute_batch("COMMIT").unwrap();
    });
    taken.recv_timeout(Duration::from_secs(30)).unwrap();
    (release, holder)
}

/// The lock is held when the op begins and let go 150 ms later: the op
/// waits it out, and the record says it was busy and for how long.
#[test]
fn a_held_lock_is_recorded_as_busy_with_its_wait() {
    let log = Arc::new(Recorded::default());
    let (dir, handles) = logged(&log);
    call(&handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
    log.clear();
    let (release, holder) = hold(&dir);
    let releaser = thread::spawn(move || {
        thread::sleep(Duration::from_millis(150));
        release.send(()).unwrap();
    });
    call(&handles, "parent", "_status", json!([])).unwrap();
    releaser.join().unwrap();
    holder.join().unwrap();
    let recorded = only(&log);
    assert_eq!(recorded.outcome, BoardOpOutcome::Ok);
    assert_eq!(recorded.busy, Some(true), "{recorded:?}");
    let busy_wait = recorded.busy_wait_us.unwrap();
    assert!(busy_wait >= 1_000, "{recorded:?}");
    assert!(recorded.lock_wait_us.unwrap() >= busy_wait, "{recorded:?}");
}

/// Held for the whole op, the lock outlasts the store's timeout: the op is
/// refused as contended after sleeping the whole timeout.
#[test]
fn a_lock_held_past_the_timeout_is_a_contended_refusal() {
    let log = Arc::new(Recorded::default());
    let (dir, handles) = logged(&log);
    call(&handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
    log.clear();
    let (release, holder) = hold(&dir);
    let answer = call(&handles, "parent", "_status", json!([]));
    release.send(()).unwrap();
    holder.join().unwrap();
    let recorded = only(&log);
    assert_eq!(refusal(&log, answer), RefusalKind::Contended);
    assert_eq!(recorded.busy, Some(true), "{recorded:?}");
    let busy_wait = recorded.busy_wait_us.unwrap();
    assert!(busy_wait >= 500_000, "{recorded:?}");
    assert!(recorded.lock_wait_us.unwrap() >= busy_wait, "{recorded:?}");
}

/// Owner decision T1: with the event log off (the default) no call is
/// measured and nothing is recorded; with it on, each call is measured
/// once.
#[test]
fn with_the_event_log_off_nothing_is_measured_or_written() {
    let dir = tempfile::TempDir::new().unwrap();
    let handles = plain(Arc::new(SqliteBoardRepository::new(&location(&dir))));
    assert!(handles.telemetry.is_none(), "off unless switched on");
    call(&handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
    call(&handles, "parent", "_status", json!([])).unwrap();
    let log = Arc::new(Recorded::default());
    let meter = Arc::new(Unmeasured::default());
    let on = SwarmBoardHandles {
        telemetry: Some(BoardTelemetry {
            log: log.clone(),
            meter: meter.clone(),
        }),
        ..handles.clone()
    };
    call(&on, "parent", "_snapshot", json!([])).unwrap();
    call(&handles, "parent", "_snapshot", json!([])).unwrap();
    assert_eq!(*meter.0.lock().unwrap(), 1, "only the call with it on");
    assert_eq!(log.ops().len(), 1);
}

/// The board is byte-identical with and without telemetry: it lives only
/// in the event log.
#[test]
fn telemetry_leaves_the_board_file_unchanged() {
    let run = |log: Option<Arc<Recorded>>| {
        let dir = tempfile::TempDir::new().unwrap();
        let repository = Arc::new(SqliteBoardRepository::new(&location(&dir)));
        let handles = match &log {
            Some(log) => with_event_log(plain(repository), log.clone()),
            None => plain(repository),
        };
        call(&handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
        call(&handles, "parent", "create_run", create_args()).unwrap();
        call(&handles, "parent", "create_run", create_args()).unwrap_err();
        call(&handles, "parent", "_snapshot", json!([])).unwrap();
        call(&handles, "ghost", "_snapshot", json!([])).unwrap_err();
        call(&handles, "parent", "_status", json!([])).unwrap();
        std::fs::read(location(&dir).database).unwrap()
    };
    let log = Arc::new(Recorded::default());
    let with = run(Some(log.clone()));
    assert_eq!(log.ops().len(), 6, "telemetry was on");
    assert!(
        with == run(None),
        "the board file differs with telemetry on"
    );
}

/// Handles recording in a real event log under `base`, as composition
/// builds them with the event log on.
fn real(base: &tempfile::TempDir, dir: &tempfile::TempDir, cap: u64) -> SwarmBoardHandles {
    let log = AuditLog::open_sync(base.path(), "cli:telemetry")
        .unwrap()
        .with_cap(cap);
    let event_log = board_op_log(true, &log).expect("the event log is on");
    let repository = Arc::new(SqliteBoardRepository::new(&location(dir)));
    with_event_log(plain(repository), event_log)
}

fn lines(base: &tempfile::TempDir) -> (String, Vec<Value>) {
    let text = std::fs::read_to_string(AuditLog::file_path(base.path(), "cli:telemetry"))
        .unwrap_or_default();
    let lines = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    (text, lines)
}

/// #2303 review H1: a board call writes its record to a real event log
/// from a plain thread, outside any async runtime, without panicking or
/// blocking on one; the record is filed under no turn.
#[test]
fn a_real_event_log_is_written_from_a_plain_thread() {
    let (base, dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let handles = real(
        &base,
        &dir,
        crate::infrastructure::persistence::audit_log::DEFAULT_CAP_BYTES,
    );
    thread::spawn(move || {
        call(&handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
        call(&handles, "parent", "_status", json!([])).unwrap();
    })
    .join()
    .expect("no runtime is needed and nothing panics");
    let (text, lines) = lines(&base);
    assert_eq!(lines.len(), 2, "{text}");
    for line in &lines {
        assert_eq!(line["event"], "swarm_op", "{line}");
        assert_eq!(line["turn"], Value::Null, "no turn is known: {line}");
    }
}

/// A log that cannot take the record leaves the op's answer as it was.
#[test]
fn a_record_the_log_cannot_take_leaves_the_answer_unchanged() {
    let (base, dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let handles = real(&base, &dir, 64);
    call(&handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
    let answer = call(&handles, "parent", "_status", json!([])).unwrap();
    let plain_dir = tempfile::tempdir().unwrap();
    let unlogged = plain(Arc::new(SqliteBoardRepository::new(&location(&plain_dir))));
    call(&unlogged, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
    assert_eq!(
        answer,
        call(&unlogged, "parent", "_status", json!([])).unwrap()
    );
    assert_eq!(lines(&base).1.len(), 0, "nothing past the cap");
}

/// The string fields a `swarm_op` line may hold: the envelope's and the
/// record's ids and kinds. Anything else would be board text.
const STRING_FIELDS: [&str; 11] = [
    "ts",
    "host",
    "session",
    "parent",
    "event",
    "op",
    "actor_ref",
    "role",
    "run_id",
    "outcome",
    "kind",
];

/// Written to a real event log, a `swarm_op` line holds no board text:
/// only allowlisted string fields, and none of the secret-shaped title,
/// constraint and criterion text the calls carried. A secret-shaped actor
/// id is redacted.
#[test]
fn a_swarm_op_line_holds_no_board_text() {
    let secret = "sk-ant-api03-CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC";
    let actor = "sk-ant-api03-DDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDD";
    let (base, dir) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let handles = real(
        &base,
        &dir,
        crate::infrastructure::persistence::audit_log::DEFAULT_CAP_BYTES,
    );
    call(&handles, actor, "bootstrap_run", json!([1, "s", null])).unwrap();
    let mut args = create_args();
    args["goal"] = json!(format!("title {secret}"));
    args["constraints"] = json!([format!("body {secret}")]);
    args["criteria"] =
        json!([{"id": "t", "kind": "command", "description": format!("evidence {secret}")}]);
    call(&handles, actor, "create_run", args.clone()).unwrap();
    call(&handles, actor, "create_run", args).unwrap_err();
    call(&handles, actor, "_snapshot", json!([])).unwrap();
    let (text, lines) = lines(&base);
    assert_eq!(lines.len(), 4, "{text}");
    assert!(
        lines.iter().any(|line| line.get("kind").is_some()),
        "a refusal's kind is checked too: {text}"
    );
    for line in &lines {
        assert_eq!(line["event"], "swarm_op", "{line}");
        let fields = line.as_object().unwrap();
        for (key, value) in fields {
            if value.is_string() {
                assert!(
                    STRING_FIELDS.contains(&key.as_str()),
                    "{key} is a string field outside the allowlist: {line}"
                );
            }
            assert!(!value.is_object() && !value.is_array(), "{key}: {line}");
        }
    }
    for leaked in [secret, actor, "sk-ant", "title", "body", "evidence"] {
        assert!(!text.contains(leaked), "{leaked} in {text}");
    }
}
