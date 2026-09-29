//! Mixed writers on one board file (#2274; S13 extends it): the Python
//! board and the Rust board take turns on the **same** SQLite file, as a
//! swarm whose members run either implementation would. A record one side
//! stored must be the bytes the other side's `encode` writes, or a
//! redelivery across implementations is refused as reused data.
use std::path::Path;

use serde_json::{Value, json};

use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::python::PyBoard;
use crate::swarm_board_diff_runs::swarm_board_diff::rust::RustBoard;

/// One board file both implementations open.
struct Mixed {
    _dir: tempfile::TempDir,
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
        "instrumented_attempts": 1, "outcome": "succeeded", "extra": [1.5, {"b": 1, "a": null}],
        "runtime": {"process_instance_id": "p", "executable_digest_pending": true}});
    let mut known = pending.clone();
    known["runtime"] = json!({"process_instance_id": "p", "executable_digest_pending": false,
        "executable_sha256": "abc"});
    (pending, known)
}

#[test]
fn a_python_recorded_request_redelivered_by_rust_is_accepted() {
    let mut board = Mixed::running();
    let (pending, known) = records();
    assert!(matches!(
        board.python_records(&pending, 3.0),
        Outcome::Ok(_)
    ));
    assert!(matches!(board.rust_records(&pending, 4.0), Outcome::Ok(_)));
    let report = board.report(5.0);
    assert_eq!(report["totals"]["requests"], json!(1));
    assert!(matches!(board.rust_records(&known, 6.0), Outcome::Ok(_)));
    let report = board.report(7.0);
    assert_eq!(
        report["recent_requests"][0]["observation"]["runtime"]["executable_sha256"],
        json!("abc")
    );
    assert!(matches!(board.python_records(&known, 8.0), Outcome::Ok(_)));
    let mut changed = known;
    changed["output_tokens"] = json!(8);
    assert_eq!(
        board.rust_records(&changed, 9.0),
        Outcome::Refused("request observation ID reused with different data".to_owned())
    );
}

#[test]
fn a_rust_recorded_request_redelivered_by_python_is_accepted() {
    let mut board = Mixed::running();
    let (pending, known) = records();
    assert!(matches!(board.rust_records(&pending, 3.0), Outcome::Ok(_)));
    assert!(matches!(
        board.python_records(&pending, 4.0),
        Outcome::Ok(_)
    ));
    assert_eq!(board.report(5.0)["totals"]["requests"], json!(1));
    assert!(matches!(board.python_records(&known, 6.0), Outcome::Ok(_)));
    assert!(matches!(board.rust_records(&known, 7.0), Outcome::Ok(_)));
    let mut changed = known;
    changed["output_tokens"] = json!(8);
    assert_eq!(
        board.python_records(&changed, 8.0),
        Outcome::Refused("request observation ID reused with different data".to_owned())
    );
    assert_eq!(board.report(9.0)["totals"]["observed_tokens"], json!(37));
}

/// A budget one side configured is the other side's budget, byte for byte:
/// configuring the same budget again on the other side writes nothing.
#[test]
fn a_budget_either_side_configured_is_the_same_budget() {
    let mut board = Mixed::running();
    let python = board
        .python
        .call("parent", "usage_budget", "[100, false]", NOW + 3.0);
    let rust = board
        .rust
        .call("parent", "usage_budget", &json!([100, false]), NOW + 4.0);
    assert_eq!(python, rust, "the same budget, unchanged by the second");
    let rust = board
        .rust
        .call("parent", "usage_budget", &json!([50, true]), NOW + 5.0);
    let python = board
        .python
        .call("parent", "usage_budget", "[50, true]", NOW + 6.0);
    assert_eq!(python, rust);
}
