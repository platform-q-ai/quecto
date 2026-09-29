//! Mixed writers on one board file (#2274; S13 extends it): the Python
//! board and the Rust board take turns on the **same** SQLite file, as a
//! swarm whose members run either implementation would. A redelivery is
//! compared by decoded value (Python's `==`), so these tests also read the
//! raw rows: each side must store the very bytes the other side's `encode`
//! (or, for the budget, `json.dumps`) writes, and neither may write where
//! the other would not.
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use serde_json::{Value, json};

use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::python::PyBoard;
use crate::swarm_board_diff_runs::swarm_board_diff::rust::RustBoard;

/// One board file both implementations open.
struct Mixed {
    _dir: tempfile::TempDir,
    database: PathBuf,
    python: PyBoard,
    rust: RustBoard,
}

impl Mixed {
    /// A running run coordinated by `parent` (created by Python), with
    /// `worker` live.
    fn running() -> Self {
        let dir = tempfile::tempdir().expect("a directory for the board");
        let checkout = dir.path().join("board");
        std::fs::create_dir_all(&checkout).expect("create the board directory");
        let database = checkout.join("swarm.sqlite");
        let mut python = PyBoard::start(&database, &checkout, dir.path());
        let rust = RustBoard::open(&database, &checkout);
        let create = json!({
            "goal": "mixed writers",
            "constraints": [],
            "criteria": [{"id": "t", "kind": "command", "description": "test"}],
            "member_limit": 5,
            "deadline": NOW + 3_600.0,
        });
        for (method, args, offset) in [
            ("create_run", create, 0.0),
            ("_admit", json!(["worker", "res-w"]), 1.0),
            ("_activate", json!(["worker", "res-w", 11, "t", null]), 2.0),
        ] {
            let outcome = python.call("parent", method, &args.to_string(), NOW + offset);
            assert!(matches!(outcome, Outcome::Ok(_)), "{method}: {outcome:?}");
        }
        assert_board(&database);
        Self {
            _dir: dir,
            database,
            python,
            rust,
        }
    }

    fn python_records(&mut self, record: &Value, offset: f64) -> Outcome {
        let args = json!([record]).to_string();
        self.python
            .call("worker", "_record_request", &args, NOW + offset)
    }

    fn rust_records(&self, record: &Value, offset: f64) -> Outcome {
        self.rust
            .call("worker", "_record_request", &json!([record]), NOW + offset)
    }

    /// The one text cell `sql` selects, read from the file as stored.
    fn stored_text(&self, sql: &str) -> String {
        let connection =
            Connection::open_with_flags(&self.database, OpenFlags::SQLITE_OPEN_READ_ONLY)
                .expect("open the shared board read-only");
        connection
            .query_row(sql, [], |row| row.get(0))
            .unwrap_or_else(|error| panic!("{sql}: {error}"))
    }

    /// The shared request's stored payload.
    fn request_payload(&self) -> String {
        self.stored_text("SELECT payload FROM request_usage WHERE request_id='shared'")
    }

    /// The stored budget payload.
    fn budget_payload(&self) -> String {
        self.stored_text("SELECT payload FROM usage_budget WHERE id=1")
    }

    /// How many events named `action` the file holds.
    fn events(&self, action: &str) -> String {
        self.stored_text(&format!(
            "SELECT CAST(count(*) AS TEXT) FROM events WHERE action='{action}'"
        ))
    }

    /// The report both sides read of the one file: identical.
    fn report(&mut self, offset: f64) -> Value {
        let python = self
            .python
            .call("parent", "usage_report", "[]", NOW + offset);
        let rust = self
            .rust
            .call("parent", "usage_report", &json!([]), NOW + offset);
        assert_eq!(python, rust, "both read the same report");
        match rust {
            Outcome::Ok(report) => report,
            other => panic!("usage_report: {other:?}"),
        }
    }
}

fn assert_board(database: &Path) {
    assert!(database.exists(), "the shared board file exists");
}

/// A record whose runtime's digest is still pending, and the same record
/// once the digest is known.
fn records() -> (Value, Value) {
    let pending = json!({"request_id": "shared", "model": "m\u{e9}", "duration_ms": 12,
        "context_input_tokens": 30, "output_tokens": 7, "input_tokens": 25,
        "instrumented_attempts": 1, "outcome": "succeeded", "extra": [1.5, 1e16, 0.1, {"b": 1, "a": null}],
        "runtime": {"process_instance_id": "p", "executable_digest_pending": true}});
    let mut known = pending.clone();
    known["runtime"] = json!({"process_instance_id": "p", "executable_digest_pending": false,
        "executable_sha256": "abc"});
    (pending, known)
}

/// `encode(pending)`, as Python's `json.dumps(sort_keys=True,
/// separators=(',', ':'))` writes it: sorted keys at every depth, `é`
/// escaped, `1e16` as `1e+16`.
const PENDING: &str = r#"{"context_input_tokens":30,"duration_ms":12,"extra":[1.5,1e+16,0.1,{"a":null,"b":1}],"input_tokens":25,"instrumented_attempts":1,"model":"m\u00e9","outcome":"succeeded","output_tokens":7,"request_id":"shared","runtime":{"executable_digest_pending":true,"process_instance_id":"p"}}"#;
/// `encode(known)`.
const KNOWN: &str = r#"{"context_input_tokens":30,"duration_ms":12,"extra":[1.5,1e+16,0.1,{"a":null,"b":1}],"input_tokens":25,"instrumented_attempts":1,"model":"m\u00e9","outcome":"succeeded","output_tokens":7,"request_id":"shared","runtime":{"executable_digest_pending":false,"executable_sha256":"abc","process_instance_id":"p"}}"#;

#[test]
fn a_python_recorded_request_redelivered_by_rust_is_accepted() {
    let mut board = Mixed::running();
    let (pending, known) = records();
    assert!(matches!(
        board.python_records(&pending, 3.0),
        Outcome::Ok(_)
    ));
    assert_eq!(board.request_payload(), PENDING, "Python's encode");
    assert!(matches!(board.rust_records(&pending, 4.0), Outcome::Ok(_)));
    assert_eq!(
        board.request_payload(),
        PENDING,
        "no digest: nothing written"
    );
    let report = board.report(5.0);
    assert_eq!(report["totals"]["requests"], json!(1));
    assert!(matches!(board.rust_records(&known, 6.0), Outcome::Ok(_)));
    assert_eq!(
        board.request_payload(),
        KNOWN,
        "Rust's encode, Python's bytes"
    );
    let report = board.report(7.0);
    assert_eq!(
        report["recent_requests"][0]["observation"]["runtime"]["executable_sha256"],
        json!("abc")
    );
    assert!(matches!(board.python_records(&known, 8.0), Outcome::Ok(_)));
    assert_eq!(
        board.request_payload(),
        KNOWN,
        "Python rewrites the same bytes"
    );
    let mut changed = known;
    changed["output_tokens"] = json!(8);
    assert_eq!(
        board.rust_records(&changed, 9.0),
        Outcome::Refused("request observation ID reused with different data".to_owned())
    );
    assert_eq!(board.request_payload(), KNOWN);
}

#[test]
fn a_rust_recorded_request_redelivered_by_python_is_accepted() {
    let mut board = Mixed::running();
    let (pending, known) = records();
    assert!(matches!(board.rust_records(&pending, 3.0), Outcome::Ok(_)));
    assert_eq!(
        board.request_payload(),
        PENDING,
        "Rust's encode, Python's bytes"
    );
    assert!(matches!(
        board.python_records(&pending, 4.0),
        Outcome::Ok(_)
    ));
    assert_eq!(
        board.request_payload(),
        PENDING,
        "no digest: nothing written"
    );
    assert_eq!(board.report(5.0)["totals"]["requests"], json!(1));
    assert!(matches!(board.python_records(&known, 6.0), Outcome::Ok(_)));
    assert_eq!(board.request_payload(), KNOWN, "Python's encode");
    assert!(matches!(board.rust_records(&known, 7.0), Outcome::Ok(_)));
    assert_eq!(
        board.request_payload(),
        KNOWN,
        "Rust rewrites the same bytes"
    );
    let mut changed = known;
    changed["output_tokens"] = json!(8);
    assert_eq!(
        board.python_records(&changed, 8.0),
        Outcome::Refused("request observation ID reused with different data".to_owned())
    );
    assert_eq!(board.report(9.0)["totals"]["observed_tokens"], json!(37));
}

/// A request of `tokens` observed tokens (context input and output), as
/// the worker's `id`.
fn measured(id: &str, tokens: u64) -> Value {
    json!({"request_id": id, "instrumented_attempts": 1, "outcome": "succeeded",
        "context_input_tokens": tokens - 5, "output_tokens": 5})
}

/// `json.dumps` of the budget Python's `configure_usage_budget` writes.
fn budget_text(limit: u64, strict_unknown: bool, warned: bool) -> String {
    format!(r#"{{"token_limit": {limit}, "strict_unknown": {strict_unknown}, "warned": {warned}}}"#)
}

/// A budget one side configured is the other side's budget, byte for byte:
/// configuring the same budget again on the other side writes nothing (no
/// second `usage-budget` event, the payload's bytes unchanged), and the
/// warning either side marks re-dumps the payload the other side wrote.
#[test]
fn a_budget_either_side_configured_is_the_same_budget() {
    let mut board = Mixed::running();
    let python = board
        .python
        .call("parent", "usage_budget", "[100, false]", NOW + 3.0);
    assert_eq!(board.events("usage-budget"), "1");
    assert_eq!(board.budget_payload(), budget_text(100, false, false));
    let rust = board
        .rust
        .call("parent", "usage_budget", &json!([100, false]), NOW + 4.0);
    assert_eq!(python, rust, "the same budget, unchanged by the second");
    assert_eq!(board.events("usage-budget"), "1", "Rust wrote nothing");
    assert_eq!(board.budget_payload(), budget_text(100, false, false));
    // Rust marks the warning on the budget Python wrote: 85 of 100.
    assert!(matches!(
        board.rust_records(&measured("r1", 85), 5.0),
        Outcome::Ok(_)
    ));
    assert_eq!(board.events("usage-warning"), "1");
    assert_eq!(board.budget_payload(), budget_text(100, false, true));

    let rust = board
        .rust
        .call("parent", "usage_budget", &json!([200, true]), NOW + 6.0);
    assert_eq!(board.events("usage-budget"), "2");
    assert_eq!(board.budget_payload(), budget_text(200, true, false));
    let python = board
        .python
        .call("parent", "usage_budget", "[200, true]", NOW + 7.0);
    assert_eq!(python, rust);
    assert_eq!(board.events("usage-budget"), "2", "Python wrote nothing");
    assert_eq!(board.budget_payload(), budget_text(200, true, false));
    // Python marks the warning on the budget Rust wrote: 165 of 200.
    assert!(matches!(
        board.python_records(&measured("r2", 80), 8.0),
        Outcome::Ok(_)
    ));
    assert_eq!(board.events("usage-warning"), "2");
    assert_eq!(board.budget_payload(), budget_text(200, true, true));
    assert_eq!(board.report(9.0)["budget"]["warned"], json!(true));
}
