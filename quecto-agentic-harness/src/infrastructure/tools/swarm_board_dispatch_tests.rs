use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::{Method, Parameter, SwarmBoardHandles, TELEMETRY_TARGET, bind, call, required};
use crate::application::swarm::dto::BoardLocation;
use crate::application::swarm::ports::{Clock, IdSource};
use crate::composition::swarm::build_swarm_board_handles_with;
use crate::domain::swarm::{BoardError, RefusalKind};
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

pub(super) fn board(now: f64) -> (tempfile::TempDir, SwarmBoardHandles) {
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
            BoardError::new(RefusalKind::Calling, message),
            "{args}"
        );
    }
}

/// The handles hold use cases, not data: their `Debug` names the type and
/// nothing else.
#[test]
fn handles_debug_names_the_type_only() {
    let (_dir, handles) = board(1_000.0);
    assert_eq!(format!("{handles:?}"), "SwarmBoardHandles { .. }");
}

#[test]
fn an_unknown_method_is_refused() {
    let (_dir, handles) = board(1_000.0);
    assert_eq!(
        call(&handles, "parent", "drop_tables", json!([])).unwrap_err(),
        BoardError::new(
            RefusalKind::Calling,
            "swarm board has no method drop_tables"
        )
    );
}

/// `_status` on a store holding no run, then on the bootstrap placeholder,
/// in Python's key order: `dict(counts, id=…, status=…, …)`.
#[test]
fn status_renders_pythons_shape_and_key_order() {
    let (_dir, handles) = board(1_000.0);
    let missing = call(&handles, "supervisor", "_status", json!([])).unwrap_err();
    assert!(
        missing
            .message()
            .starts_with("coordination store missing at "),
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

/// P3 (#2270 review L3): `bootstrap_run`'s arguments bind as Python's
/// `sqlite3` binds them, `True` as 1, and the column affinity decides what
/// is stored: the snapshot shows it.
#[test]
fn bootstrap_arguments_bind_as_python_binds_them() {
    for (args, pid, started, socket) in [
        (json!([true, 5, null]), "1", r#""5""#, "null"),
        (json!(["7", 1.5, false]), "7", r#""1.5""#, r#""0""#),
        (json!([7.5, "Mon", "/s"]), "7.5", r#""Mon""#, r#""/s""#),
        (json!(["abc", null, 3]), r#""abc""#, "null", r#""3""#),
    ] {
        let (_dir, handles) = board(1_000.0);
        call(&handles, "parent", "bootstrap_run", args.clone()).unwrap();
        let snapshot = call(&handles, "parent", "_snapshot", json!([])).unwrap();
        let row = &snapshot["members"][0];
        let text = |key: &str| serde_json::to_string(&row[key]).unwrap();
        assert_eq!(
            [text("pid"), text("started"), text("socket")],
            [pid, started, socket],
            "{args}"
        );
    }
    // A list binds as nothing: the store refuses it and keeps no row.
    let (_dir, handles) = board(1_000.0);
    let refused = call(&handles, "parent", "bootstrap_run", json!([[1], "s", null])).unwrap_err();
    assert!(
        refused
            .message()
            .starts_with("coordination store unavailable or contended: "),
        "{refused}"
    );
    call(&handles, "parent", "bootstrap_run", json!([7, "s", null])).unwrap();
    let snapshot = call(&handles, "parent", "_snapshot", json!([])).unwrap();
    assert_eq!(snapshot["members"].as_array().map(Vec::len), Some(1));
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
        BoardError::new(
            RefusalKind::Invalid,
            "member limit must be 1 through 25 including coordinator"
        )
    );
    assert_eq!(
        call(&handles, "parent", "create_run", create_args()).unwrap_err(),
        BoardError::new(
            RefusalKind::RunExists,
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
pub(super) fn captured(records: usize, scenario: impl Fn(&SwarmBoardHandles)) -> String {
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
    // A read-only method records at DEBUG, anything else at INFO (#2270
    // round-3 review N3).
    let expected = [
        (
            " INFO ",
            "op=\"bootstrap_run\"",
            "outcome=\"ok\"",
            "decision=\"created\"",
        ),
        (
            " INFO ",
            "op=\"create_run\"",
            "outcome=\"ok\"",
            "decision=\"over_setup\"",
        ),
        (
            " INFO ",
            "op=\"create_run\"",
            "outcome=\"refused\"",
            "decision=\"none\"",
        ),
        (
            "DEBUG ",
            "op=\"_snapshot\"",
            "outcome=\"ok\"",
            "decision=\"read\"",
        ),
        (
            "DEBUG ",
            "op=\"_status\"",
            "outcome=\"ok\"",
            "decision=\"read\"",
        ),
        (
            " INFO ",
            "op=\"unknown\"",
            "outcome=\"refused\"",
            "decision=\"none\"",
        ),
    ];
    assert_eq!(records.len(), expected.len(), "{log}");
    for (record, (level, op, outcome, decision)) in records.iter().zip(expected) {
        for field in [level, op, outcome, decision, "duration_us="] {
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

/// A running run of three coordinated by `parent` (#2271).
pub(super) fn running(handles: &SwarmBoardHandles) {
    call(handles, "parent", "create_run", create_args()).unwrap();
}

/// `_admit` answers the member's row as `dict(row)`, in table order; the
/// other membership methods answer `null`.
#[test]
fn membership_methods_render_pythons_shape() {
    let (_dir, handles) = board(1_000.0);
    running(&handles);
    let row = call(&handles, "parent", "_admit", json!(["worker", "r"])).unwrap();
    assert_eq!(
        serde_json::to_string(&row).unwrap(),
        r#"{"id":"worker","reservation":"r","status":"reserved","pid":null,"started":null,"socket":null,"launcher":"parent"}"#
    );
    for (method, args) in [
        ("_record_launch", json!(["worker", "r", 7, "t"])),
        (
            "_activate",
            json!({"member": "worker", "reservation": "r", "pid": 7, "started": "t", "socket": null}),
        ),
        ("_socket", json!(["/p.sock"])),
    ] {
        assert_eq!(
            call(&handles, "parent", method, args).unwrap(),
            Value::Null,
            "{method}"
        );
    }
    call(&handles, "parent", "_admit", json!(["spare", "s"])).unwrap();
    assert_eq!(
        call(&handles, "parent", "_release_unlaunched", json!(["spare"])).unwrap(),
        Value::Null
    );
    // The spare's death freed its place: the join is admitted into it.
    assert_eq!(
        call(&handles, "joiner", "bootstrap_join", json!([9, "t", null])).unwrap(),
        Value::Null
    );
    assert_eq!(
        call(&handles, "late", "bootstrap_join", json!([10, "t", null])).unwrap_err(),
        BoardError::new(
            RefusalKind::MemberLimit,
            "swarm limit 3, current usage 3; reuse the existing pool"
        )
    );
}

/// Every membership argument reaches the board as the member passed it
/// (#2271 round-1 review M1): Python's `sqlite3` binds the member,
/// reservation, pid, start time and socket untyped, the column affinity
/// decides what is stored, and the board compares with Python's `==`. No
/// type is refused as calling syntax; only an argument `sqlite3` cannot
/// bind is refused, as Python's store refuses it.
#[test]
fn membership_arguments_reach_the_board_as_given() {
    let (_dir, handles) = board(1_000.0);
    running(&handles);
    let row = call(&handles, "parent", "_admit", json!(["w", 5])).unwrap();
    assert_eq!(row["reservation"], json!("5"), "{row}");
    assert_eq!(
        call(
            &handles,
            "parent",
            "_record_launch",
            json!(["w", null, 7, "t"])
        )
        .unwrap_err(),
        BoardError::new(RefusalKind::StaleToken, "stale launch reservation")
    );
    for (args, answer) in [
        (json!(["w", "5", "7", "t", null]), Ok(Value::Null)),
        (json!(["w", "5", 7.0, "t", null]), Ok(Value::Null)),
        (
            json!(["w", "5", true, "t", null]),
            Err(BoardError::new(
                RefusalKind::LaunchConflict,
                "member already active in a different process",
            )),
        ),
        (
            json!(["w", 5, 7, "t", null]),
            Err(BoardError::new(
                RefusalKind::StaleToken,
                "unknown or stale launch reservation",
            )),
        ),
    ] {
        assert_eq!(
            call(&handles, "parent", "_activate", args.clone()),
            answer,
            "{args}"
        );
    }
    assert_eq!(
        call(&handles, "w", "_socket", json!([5])).unwrap(),
        Value::Null
    );
    let row = call(&handles, "parent", "_admit", json!(["n", null])).unwrap();
    assert_eq!(row["reservation"], Value::Null, "{row}");
    assert_eq!(
        call(
            &handles,
            "parent",
            "_activate",
            json!(["n", null, [1], "t", null])
        )
        .unwrap_err(),
        BoardError::new(
            RefusalKind::Invalid,
            "coordination store unavailable or contended: \
             Error binding parameter 1: type 'list' is not supported"
        )
    );
    assert_eq!(
        call(&handles, "parent", "_release_unlaunched", json!([5])).unwrap_err(),
        BoardError::new(
            RefusalKind::WrongState,
            "only an unlaunched reservation may be released"
        )
    );
    let snapshot = call(&handles, "parent", "_snapshot", json!([])).unwrap();
    let worker = &snapshot["members"][1];
    assert_eq!(
        (&worker["pid"], &worker["started"], &worker["socket"]),
        (&json!(7), &json!("t"), &json!("5")),
        "{snapshot}"
    );
    assert_eq!(
        call(&handles, "parent", "_socket", json!([])).unwrap_err(),
        BoardError::new(
            RefusalKind::Calling,
            "_socket: missing required argument socket"
        )
    );
    assert_eq!(
        call(&handles, "parent", "_admit", json!([5, "r"])).unwrap_err(),
        BoardError::new(
            RefusalKind::Invalid,
            "member must be nonempty and at most 128 bytes"
        )
    );
}

/// Each membership call leaves one INFO record with its decision, and no
/// argument text: a secret-shaped reservation, start time or socket never
/// reaches the log (#2271).
#[test]
fn membership_calls_record_their_decisions_without_argument_text() {
    let secret = "sk-ant-api03-CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC";
    let other = "sk-ant-api03-DDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDD";
    let log = captured(9, |handles| {
        running(handles);
        call(handles, "parent", "_admit", json!(["worker", secret])).unwrap();
        call(handles, "parent", "_admit", json!(["worker", secret])).unwrap();
        call(
            handles,
            "parent",
            "_record_launch",
            json!(["worker", secret, 7, secret]),
        )
        .unwrap();
        call(
            handles,
            "parent",
            "_activate",
            json!(["worker", secret, 7, secret, secret]),
        )
        .unwrap();
        call(handles, "worker", "_socket", json!([secret])).unwrap();
        call(handles, "parent", "_release_unlaunched", json!(["worker"])).unwrap_err();
        call(
            handles,
            "joiner",
            "bootstrap_join",
            json!([8, secret, secret, other]),
        )
        .unwrap();
        call(
            handles,
            "joiner",
            "bootstrap_join",
            json!([8, secret, secret]),
        )
        .unwrap();
    });
    let records: Vec<&str> = log
        .lines()
        .filter(|line| line.contains(TELEMETRY_TARGET))
        .collect();
    let expected = [
        ("create_run", "ok", "fresh"),
        ("_admit", "ok", "reserved"),
        ("_admit", "ok", "retry"),
        ("_record_launch", "ok", "recorded"),
        ("_activate", "ok", "activated"),
        ("_socket", "ok", "registered"),
        ("_release_unlaunched", "refused", "none"),
        ("bootstrap_join", "ok", "admitted"),
        ("bootstrap_join", "ok", "already_live"),
    ];
    assert_eq!(records.len(), expected.len(), "{log}");
    for (record, (op, outcome, decision)) in records.iter().zip(expected) {
        for field in [
            " INFO ".to_owned(),
            format!("op=\"{op}\""),
            format!("outcome=\"{outcome}\""),
            format!("decision=\"{decision}\""),
            "duration_us=".to_owned(),
        ] {
            assert!(record.contains(&field), "{field} missing from {record}");
        }
    }
    assert!(!log.contains(secret) && !log.contains(other), "{log}");
    assert!(!log.contains("sk-ant"), "{log}");
}

/// Why `call` takes its arguments from `py_json::decode`: serde's default
/// float parser rounds `2^53 + 1` up, Python's `json.loads` rounds it to
/// even, and `python_equal` then answers differently against the integer
/// Python compares it with.
#[test]
fn serde_float_parsing_would_change_python_equal() {
    use crate::domain::swarm::python_equal;
    use crate::infrastructure::persistence::swarm_board::py_json;

    let literal = "9007199254740993.0";
    let python = py_json::decode(literal).unwrap().to_value().unwrap();
    let serde: Value = serde_json::from_str(literal).unwrap();
    let stored = json!(9_007_199_254_740_992_i64);
    assert_eq!(python.as_f64(), Some(9_007_199_254_740_992.0));
    assert!(python_equal(&python, &stored));
    assert!(
        !python_equal(&serde, &stored),
        "serde parsed {serde}; if it now rounds correctly, the S13/S14 \
         caution on `call` may be relaxed"
    );
}
