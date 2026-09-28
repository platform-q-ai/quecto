use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::{Method, Parameter, SwarmBoardHandles, TELEMETRY_TARGET, bind, call, required};
use crate::application::swarm::dto::BoardLocation;
use crate::application::swarm::ports::{Clock, IdSource};
use crate::composition::swarm::build_swarm_board_handles_with;
use crate::domain::swarm::BoardError;
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

fn board(now: f64) -> (tempfile::TempDir, SwarmBoardHandles) {
    let dir = tempfile::TempDir::new().unwrap();
    let location = BoardLocation {
        database: dir.path().join("swarm.sqlite"),
        checkout: dir.path().to_path_buf(),
    };
    let handles = build_swarm_board_handles_with(
        Arc::new(SqliteBoardRepository::new(&location)),
        Arc::new(Fixed(now)),
        Arc::new(Counter::default()),
    );
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

const SIGNATURE: [Parameter; 3] = [
    required("first"),
    required("second"),
    Parameter {
        name: "third",
        default: Some(|| json!(25)),
    },
];

#[test]
fn arguments_bind_positionally_by_name_and_by_default() {
    let bound = |args| bind(Method::CreateRun, &SIGNATURE, args);
    assert_eq!(
        bound(json!([1, 2, 3])).unwrap(),
        [json!(1), json!(2), json!(3)]
    );
    assert_eq!(
        bound(json!([1, 2])).unwrap(),
        [json!(1), json!(2), json!(25)]
    );
    assert_eq!(
        bound(json!({"second": 2, "first": 1})).unwrap(),
        [json!(1), json!(2), json!(25)]
    );
    assert_eq!(
        bound(json!({"third": null, "first": 1, "second": 2})).unwrap(),
        [json!(1), json!(2), Value::Null],
        "an explicit null is bound, not replaced by the default"
    );
    let refusals = [
        (
            json!([1, 2, 3, 4]),
            "create_run: takes 3 arguments, 4 given",
        ),
        (
            json!({"first": 1, "fourth": 4}),
            "create_run: unexpected argument fourth",
        ),
        (json!([1]), "create_run: missing required argument second"),
        (
            json!({"second": 2}),
            "create_run: missing required argument first",
        ),
        (
            json!("positional"),
            "create_run: arguments must be a JSON array or object",
        ),
        (
            Value::Null,
            "create_run: arguments must be a JSON array or object",
        ),
    ];
    for (args, message) in refusals {
        assert_eq!(
            bound(args.clone()).unwrap_err(),
            BoardError::new(message),
            "{args}"
        );
    }
}

#[test]
fn an_unknown_method_is_refused() {
    let (_dir, handles) = board(1_000.0);
    assert_eq!(
        call(&handles, "parent", "drop_tables", json!([])).unwrap_err(),
        BoardError::new("swarm board has no method drop_tables")
    );
}

/// `_status` on a store holding no run, then on the bootstrap placeholder,
/// in Python's key order: `dict(counts, id=…, status=…, …)`.
#[test]
fn status_renders_pythons_shape_and_key_order() {
    let (_dir, handles) = board(1_000.0);
    let missing = call(&handles, "supervisor", "_status", json!([])).unwrap_err();
    assert!(
        missing.0.starts_with("coordination store missing at "),
        "{missing}"
    );
    call(
        &handles,
        "parent",
        "bootstrap_run",
        json!([7, "Mon 7", "/run/p.sock"]),
    )
    .unwrap();
    let status = call(&handles, "supervisor", "_status", json!([])).unwrap();
    assert_eq!(
        serde_json::to_string(&status).unwrap(),
        format!(
            r#"{{"members_without_claim":0,"members_dead":0,"id":"{:032x}","status":"setup","deadline":0.0,"coordinator":"parent","outcome":null}}"#,
            1
        )
    );
}

#[test]
fn snapshot_renders_every_member_row_as_a_dict() {
    let (_dir, handles) = board(1_000.0);
    call(
        &handles,
        "parent",
        "bootstrap_run",
        json!([7, "Mon 7", null]),
    )
    .unwrap();
    call(&handles, "parent", "create_run", create_args()).unwrap();
    let snapshot = call(&handles, "parent", "_snapshot", json!({})).unwrap();
    assert_eq!(
        serde_json::to_string(&snapshot).unwrap(),
        format!(
            r#"{{"status":"running","coordinator":"parent","outcome":null,"control_generation":0,"deadline":4600.0,"members":[{{"id":"parent","reservation":"{:032x}","status":"live","pid":7,"started":"Mon 7","socket":null,"launcher":null}}]}}"#,
            2
        )
    );
}

#[test]
fn bootstrap_arguments_are_typed_at_the_boundary() {
    let (_dir, handles) = board(1_000.0);
    for (args, message) in [
        (
            json!(["7", "Mon", "/s"]),
            "bootstrap_run: pid must be an integer or null",
        ),
        (
            json!([7.5, "Mon", "/s"]),
            "bootstrap_run: pid must be an integer or null",
        ),
        (
            json!([7, 1, "/s"]),
            "bootstrap_run: started must be a string or null",
        ),
        (
            json!([7, "Mon", false]),
            "bootstrap_run: socket must be a string or null",
        ),
    ] {
        assert_eq!(
            call(&handles, "parent", "bootstrap_run", args.clone()).unwrap_err(),
            BoardError::new(message),
            "{args}"
        );
    }
}

#[test]
fn create_run_returns_null_and_board_refusals_keep_their_text() {
    let (_dir, handles) = board(1_000.0);
    assert_eq!(
        call(&handles, "parent", "create_run", create_args()).unwrap(),
        Value::Null
    );
    let mut again = create_args();
    again["member_limit"] = json!(true);
    assert_eq!(
        call(&handles, "parent", "create_run", again).unwrap_err(),
        BoardError::new("member limit must be 1 through 25 including coordinator")
    );
    assert_eq!(
        call(&handles, "parent", "create_run", create_args()).unwrap_err(),
        BoardError::new(
            "only the setup coordinator can create this run; existing runs cannot be reset"
        )
    );
}

#[derive(Clone, Default)]
struct CapturedLog(Arc<Mutex<String>>);

impl std::io::Write for CapturedLog {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap()
            .push_str(&String::from_utf8_lossy(buf));
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CapturedLog {
    type Writer = CapturedLog;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

/// The board calls `scenario` makes, as the `fmt` subscriber writes them.
///
/// `tracing`'s global level hint is rebuilt without a lock whenever any
/// test thread creates a subscriber, so a concurrent test's WARN-level
/// subscriber can leave it below INFO for a moment and filter these events
/// out before any subscriber sees them. A capture holding fewer board
/// records than `records` is that race, never an answer: the scenario
/// reruns on a fresh board (at most five times) after the interest cache is
/// rebuilt, and the last capture is returned for the caller to judge.
fn captured(records: usize, scenario: impl Fn(&SwarmBoardHandles)) -> String {
    let mut log = String::new();
    for _ in 0..5 {
        let (_dir, handles) = board(1_000.0);
        let logs = CapturedLog::default();
        let sink = logs.0.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(logs)
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            tracing::callsite::rebuild_interest_cache();
            scenario(&handles);
        });
        log = sink.lock().unwrap().clone();
        if log.matches(TELEMETRY_TARGET).count() >= records {
            break;
        }
    }
    log
}

/// Every call leaves exactly one record on the board's target: the op, the
/// member id, the outcome, the decision and the duration. Argument text
/// (here a goal and a constraint holding secret-shaped keys) never appears.
#[test]
fn each_call_records_one_telemetry_event_without_argument_text() {
    let secret = "sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let log = captured(6, |handles| {
        call(handles, "parent", "bootstrap_run", json!([1, "s", "/p"])).unwrap();
        let mut args = create_args();
        args["goal"] = json!(format!("use {secret}"));
        args["constraints"] = json!([format!("key {secret}")]);
        call(handles, "parent", "create_run", args.clone()).unwrap();
        call(handles, "parent", "create_run", args).unwrap_err();
        call(handles, "parent", "_snapshot", json!([])).unwrap();
        call(handles, "supervisor", "_status", json!([])).unwrap();
        call(handles, "parent", "no_such_method", json!([])).unwrap_err();
    });
    let records: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(TELEMETRY_TARGET))
        .collect();
    let expected = [
        (
            "op=\"bootstrap_run\"",
            "outcome=\"ok\"",
            "decision=\"created\"",
        ),
        (
            "op=\"create_run\"",
            "outcome=\"ok\"",
            "decision=\"over_setup\"",
        ),
        (
            "op=\"create_run\"",
            "outcome=\"refused\"",
            "decision=\"none\"",
        ),
        ("op=\"_snapshot\"", "outcome=\"ok\"", "decision=\"read\""),
        ("op=\"_status\"", "outcome=\"ok\"", "decision=\"read\""),
        ("op=\"unknown\"", "outcome=\"refused\"", "decision=\"none\""),
    ];
    assert_eq!(records.len(), expected.len(), "{log}");
    for (record, (op, outcome, decision)) in records.iter().zip(expected) {
        for field in [op, outcome, decision, "duration_us="] {
            assert!(record.contains(field), "{field} missing from {record}");
        }
    }
    assert!(records[4].contains("member=\"supervisor\""), "{log}");
    assert!(!log.contains(secret), "{log}");
    assert!(!log.contains("sk-ant"), "{log}");
    assert!(!log.contains("use "), "argument text never logged: {log}");
    assert!(!log.contains("no_such_method"), "{log}");
}

#[test]
fn a_secret_shaped_member_id_is_redacted_in_telemetry() {
    let secret = "sk-ant-api03-BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";
    let log = captured(1, |handles| {
        call(handles, secret, "_snapshot", json!([])).unwrap_err();
    });
    assert!(log.contains("op=\"_snapshot\""), "{log}");
    assert!(!log.contains(secret), "{log}");
}
