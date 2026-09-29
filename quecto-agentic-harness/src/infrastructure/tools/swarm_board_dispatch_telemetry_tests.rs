//! #2303: each board call records exactly one `swarm_op` in the event log
//! (only when it is switched on), with its refusal kind, its real lock
//! wait, and no board text.
use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

use super::{BOARD_OPS, SwarmBoardHandles, call};
use crate::application::audit::ports::AuditSink;
use crate::application::swarm::dto::BoardLocation;
use crate::application::swarm::ports::{BoardRepository, BoardWork, Clock, IdSource};
use crate::composition::swarm::{build_swarm_board_handles_with, with_event_log};
use crate::domain::audit::AuditEvent;
use crate::domain::error::DomainError;
use crate::domain::swarm::{
    BoardError, BoardOpObservation, BoardOpOutcome, BoardRole, RefusalKind,
};
use crate::infrastructure::persistence::audit_log::AuditLog;
use crate::infrastructure::persistence::swarm_board::meter;
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
struct Recorded(Mutex<Vec<AuditEvent>>);

impl AuditSink for Recorded {
    fn emit(
        &self,
        _turn: u32,
        event: AuditEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>> {
        self.0.lock().unwrap().push(event);
        Box::pin(async { Ok(()) })
    }
}

impl Recorded {
    fn ops(&self) -> Vec<BoardOpObservation> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                AuditEvent::SwarmOp(observation) => Some(observation.clone()),
                _ => None,
            })
            .collect()
    }
}

/// Whether each board transaction ran inside a metered call.
struct Probe {
    inner: SqliteBoardRepository,
    metered: Mutex<Vec<bool>>,
}

impl BoardRepository for Probe {
    fn atomic(&self, create: bool, work: &mut BoardWork<'_>) -> Result<(), BoardError> {
        self.metered.lock().unwrap().push(meter::active());
        self.inner.atomic(create, work)
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
            log.0.lock().unwrap().clear();
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

fn refusal(log: &Recorded, answer: Result<Value, BoardError>) -> RefusalKind {
    let error = answer.expect_err("the board refuses");
    let recorded = only(log);
    log.0.lock().unwrap().clear();
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
    log.0.lock().unwrap().clear();
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
fn an_answered_op_records_its_run_role_and_size() {
    let log = Arc::new(Recorded::default());
    let (_dir, handles) = logged(&log);
    call(&handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
    log.0.lock().unwrap().clear();
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
    assert!(!recorded.busy);
    assert!(!recorded.cursor_moved);
    assert!(
        recorded.duration_us >= recorded.lock_wait_us,
        "{recorded:?}"
    );
    assert_eq!((recorded.task_id, recorded.message_id), (None, None));
}

/// A second connection holds `BEGIN IMMEDIATE` on the board for `hold`,
/// then commits. Returns once the lock is held.
fn hold(dir: &tempfile::TempDir, hold: Duration) -> thread::JoinHandle<()> {
    let path = location(dir).database;
    let (held, taken) = mpsc::channel();
    let holder = thread::spawn(move || {
        let connection = rusqlite::Connection::open(path).unwrap();
        connection.execute_batch("BEGIN IMMEDIATE").unwrap();
        held.send(()).unwrap();
        thread::sleep(hold);
        connection.execute_batch("COMMIT").unwrap();
    });
    taken.recv_timeout(Duration::from_secs(10)).unwrap();
    holder
}

/// A second connection holds the write lock: the op waits it out, and the
/// record says so.
#[test]
fn a_held_lock_is_recorded_as_busy_with_its_wait() {
    let log = Arc::new(Recorded::default());
    let (dir, handles) = logged(&log);
    call(&handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
    log.0.lock().unwrap().clear();
    let holder = hold(&dir, Duration::from_millis(100));
    call(&handles, "parent", "_status", json!([])).unwrap();
    holder.join().unwrap();
    let recorded = only(&log);
    assert_eq!(recorded.outcome, BoardOpOutcome::Ok);
    assert!(recorded.busy, "{recorded:?}");
    assert!(recorded.lock_wait_us >= 100_000, "{recorded:?}");
}

/// Held past the store's timeout, the op is refused as contended.
#[test]
fn a_lock_held_past_the_timeout_is_a_contended_refusal() {
    let log = Arc::new(Recorded::default());
    let (dir, handles) = logged(&log);
    call(&handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
    log.0.lock().unwrap().clear();
    let holder = hold(&dir, Duration::from_millis(900));
    let answer = call(&handles, "parent", "_status", json!([]));
    holder.join().unwrap();
    let recorded = only(&log);
    assert_eq!(refusal(&log, answer), RefusalKind::Contended);
    assert!(recorded.busy, "{recorded:?}");
    assert!(recorded.lock_wait_us >= 500_000, "{recorded:?}");
}

/// Owner decision T1: with the event log off (the default) no call is
/// metered and nothing is recorded; with it on, every transaction is.
#[test]
fn with_the_event_log_off_nothing_is_measured_or_written() {
    let dir = tempfile::TempDir::new().unwrap();
    let probe = Arc::new(Probe {
        inner: SqliteBoardRepository::new(&location(&dir)),
        metered: Mutex::new(Vec::new()),
    });
    let handles = plain(probe.clone());
    assert!(handles.event_log.is_none(), "off unless switched on");
    call(&handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
    call(&handles, "parent", "_status", json!([])).unwrap();
    call(&handles, "parent", "_snapshot", json!([])).unwrap();
    assert!(
        probe.metered.lock().unwrap().iter().all(|metered| !metered),
        "{:?}",
        probe.metered
    );
    let log = Arc::new(Recorded::default());
    let handles = with_event_log(handles, log.clone());
    probe.metered.lock().unwrap().clear();
    call(&handles, "parent", "_snapshot", json!([])).unwrap();
    let metered = probe.metered.lock().unwrap().clone();
    assert!(
        !metered.is_empty() && metered.iter().all(|metered| *metered),
        "{metered:?}"
    );
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

/// The string fields a `swarm_op` line may hold: the envelope's and the
/// record's ids and kinds. Anything else would be board text.
const STRING_FIELDS: [&str; 10] = [
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
];

/// Written to a real event log, a `swarm_op` line holds no board text:
/// only allowlisted string fields, and none of the secret-shaped title,
/// constraint and criterion text the calls carried. A secret-shaped actor
/// id is redacted.
#[tokio::test]
async fn a_swarm_op_line_holds_no_board_text() {
    let secret = "sk-ant-api03-CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC";
    let actor = "sk-ant-api03-DDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDD";
    let base = tempfile::TempDir::new().unwrap();
    let dir = tempfile::TempDir::new().unwrap();
    let sink: Arc<dyn AuditSink> =
        Arc::new(AuditLog::open_sync(base.path(), "cli:telemetry").unwrap());
    let repository = Arc::new(SqliteBoardRepository::new(&location(&dir)));
    let handles = with_event_log(plain(repository), sink);
    tokio::task::spawn_blocking(move || {
        call(&handles, actor, "bootstrap_run", json!([1, "s", null])).unwrap();
        let mut args = create_args();
        args["goal"] = json!(format!("title {secret}"));
        args["constraints"] = json!([format!("body {secret}")]);
        args["criteria"] =
            json!([{"id": "t", "kind": "command", "description": format!("evidence {secret}")}]);
        call(&handles, actor, "create_run", args.clone()).unwrap();
        call(&handles, actor, "create_run", args).unwrap_err();
        call(&handles, actor, "_snapshot", json!([])).unwrap();
    })
    .await
    .unwrap();
    let text = std::fs::read_to_string(AuditLog::file_path(base.path(), "cli:telemetry")).unwrap();
    let lines: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 4, "{text}");
    for line in &lines {
        assert_eq!(line["event"], "swarm_op", "{line}");
        let fields = line.as_object().unwrap();
        for (key, value) in fields {
            if value.is_string() {
                assert!(
                    STRING_FIELDS.contains(&key.as_str()) || key == "kind",
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
