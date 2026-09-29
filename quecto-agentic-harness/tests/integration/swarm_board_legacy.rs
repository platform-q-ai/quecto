//! A board the last Python board wrote opens under the Rust board and
//! completes its run (#2283; epic #2265's acceptance criterion "a board
//! created by the last Python-only release opens and completes").
//!
//! `tests/fixtures/swarm_board/legacy_python_board.sqlite` was written by
//! the Python board's own sources (`swarm.py` and the modules it imports,
//! as the harness ran them before #2282) through [`python_steps`], before
//! #2283 deleted them: a run stopped mid-way, paused, with live members,
//! tasks claimed, blocked and waiting on a dependency, reserved files, a
//! read and acknowledged message and one still unread, criterion evidence
//! at an earlier revision, a usage budget and a recorded request. The
//! fixture is binary data, never regenerated: the Python board is gone.
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::rust::RustBoard;

/// The board file the Python board left.
pub(crate) const LEGACY_BOARD: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/swarm_board/legacy_python_board.sqlite"
);

/// One call: the member, the method and its arguments, at `NOW + offset`.
/// `{token1}` and `{token3}` in the arguments are the claim tokens tasks 1
/// and 3 were claimed with.
pub(crate) struct Call(pub &'static str, pub &'static str, pub Value, pub f64);

/// The calls the Python board ran to leave the fixture, in order, from an
/// empty checkout (kept as the record of what the fixture holds).
pub(crate) fn python_steps() -> Vec<Call> {
    vec![
        Call(
            "parent",
            "create_run",
            json!([
                "legacy board",
                ["keep the schema"],
                [
                    {"id": "tests", "kind": "command", "description": "acceptance tests pass"},
                    {"id": "review", "kind": "review", "description": "independent review"}
                ],
                5,
                NOW + 86_400.0
            ]),
            0.0,
        ),
        Call("parent", "_admit", json!(["worker", "res-w"]), 1.0),
        Call(
            "parent",
            "_activate",
            json!(["worker", "res-w", 101, "start-w", "/tmp/w.sock"]),
            2.0,
        ),
        Call("parent", "_admit", json!(["helper", "res-h"]), 3.0),
        Call(
            "parent",
            "_activate",
            json!(["helper", "res-h", 102, "start-h", null]),
            4.0,
        ),
        Call(
            "worker",
            "task_create",
            json!(["t1", "first", ["tests pass"], []]),
            5.0,
        ),
        Call(
            "worker",
            "task_create",
            json!(["t2", "second", ["docs"], [1]]),
            6.0,
        ),
        Call(
            "helper",
            "task_create",
            json!(["t3", "third", ["lint"]]),
            7.0,
        ),
        Call("worker", "claim", json!([1]), 8.0),
        Call(
            "worker",
            "reserve",
            json!([1, "{token1}", ["src/lib.rs", "docs/a.md"]]),
            9.0,
        ),
        Call("helper", "claim", json!([3]), 10.0),
        Call(
            "helper",
            "block",
            json!([3, "{token3}", "waiting on review"]),
            11.0,
        ),
        Call(
            "worker",
            "send",
            json!(["m1", "parent", "first is under way"]),
            12.0,
        ),
        Call("parent", "inbox", json!([]), 13.0),
        Call("parent", "ack", json!([1]), 14.0),
        Call(
            "parent",
            "send",
            json!(["m2", "helper", "unblock when ready"]),
            15.0,
        ),
        Call(
            "worker",
            "evidence",
            json!(["tests", "report", "R1", "command", true]),
            16.0,
        ),
        Call("parent", "usage_budget", json!([100_000, false]), 17.0),
        Call(
            "worker",
            "_record_request",
            json!([{"request_id": "q1", "instrumented_attempts": 1, "outcome": "succeeded", "context_input_tokens": 40, "output_tokens": 5}]),
            18.0,
        ),
        Call("parent", "pause", json!(["operator hold"]), 19.0),
    ]
}

/// `{token1}` and `{token3}` in `args` replaced by `tokens`.
pub(crate) fn with_tokens(args: &Value, tokens: &[(String, String)]) -> Value {
    let mut text = args.to_string();
    for (name, token) in tokens {
        text = text.replace(&format!("{{{name}}}"), token);
    }
    serde_json::from_str(&text).expect("arguments stay JSON")
}

/// A copy of the fixture in a checkout of its own.
fn legacy_checkout() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("a directory for the checkout");
    let database = dir.path().join("swarm.sqlite");
    std::fs::copy(LEGACY_BOARD, &database).expect("copy the legacy board");
    (dir, database)
}

/// One text cell `sql` selects from the board.
fn stored(database: &Path, sql: &str) -> String {
    rusqlite::Connection::open(database)
        .expect("the board opens")
        .query_row(sql, [], |row| row.get(0))
        .unwrap_or_else(|error| panic!("{sql}: {error}"))
}

/// Where the Rust board's id counter starts: past every id the Python
/// board drew (it drew one per run, reservation, claim and file token,
/// counting from 1, in the twenty calls of [`python_steps`]), so no id
/// the Rust board draws repeats one the file holds.
const DRAWN_BY_PYTHON: u64 = 1_000;

fn granted(board: &RustBoard, member: &str, method: &str, args: Value, offset: f64) -> Value {
    match board.call(member, method, &args, NOW + offset) {
        Outcome::Ok(value) => value,
        other => panic!("{member} {method} {args}: {other:?}"),
    }
}

#[test]
fn a_legacy_python_board_opens_and_completes_under_rust() {
    let (dir, database) = legacy_checkout();
    assert!(
        stored(&database, "SELECT status FROM run") == "paused",
        "the Python board left a paused run"
    );
    let tokens = [
        (
            "token1".to_owned(),
            stored(&database, "SELECT token FROM tasks WHERE id=1"),
        ),
        (
            "token3".to_owned(),
            stored(&database, "SELECT token FROM tasks WHERE id=3"),
        ),
    ];
    let board = RustBoard::open_after(&database, dir.path(), DRAWN_BY_PYTHON);
    let call = |member: &str, method: &str, args: Value, offset: f64| {
        granted(&board, member, method, with_tokens(&args, &tokens), offset)
    };
    // What Python left reads back: the tasks, the reservations, the
    // unread message, the budget and the recorded request.
    let tasks = call("parent", "tasks", json!([]), 20.0);
    let statuses: Vec<&str> = tasks
        .as_array()
        .expect("a task list")
        .iter()
        .map(|task| task["status"].as_str().expect("a status"))
        .collect();
    assert_eq!(statuses, ["claimed", "blocked", "blocked"], "{tasks}");
    let owners = call("parent", "file_owners", json!([]), 20.1);
    assert_eq!(owners.as_array().map(Vec::len), Some(2), "{owners}");
    let report = call("parent", "usage_report", json!([]), 20.2);
    assert_eq!(report["totals"]["requests"], json!(1), "{report}");
    assert_eq!(report["budget"]["token_limit"], json!(100_000), "{report}");

    // The supervisor resumes the run; a Python-recorded request replays.
    call("parent", "_resume_external", json!([]), 21.0);
    let replayed = call(
        "worker",
        "task_create",
        json!(["t1", "first", ["tests pass"], []]),
        22.0,
    );
    assert_eq!(replayed["id"], json!(1), "the stored result: {replayed}");
    let inbox = call("helper", "inbox", json!([]), 23.0);
    assert_eq!(inbox[0]["body"], json!("unblock when ready"), "{inbox}");
    call("helper", "ack", json!([2]), 23.1);

    // Every task is finished at the final revision, R2.
    for (member, method, args, offset) in [
        (
            "helper",
            "unblock",
            json!([3, "{token3}", "reviewed"]),
            24.0,
        ),
        (
            "helper",
            "submit",
            json!([3, "{token3}", [{"artifact": "lint", "revision": "R2"}]]),
            24.1,
        ),
        ("parent", "verify_task", json!([3, "{token3}", "R2"]), 24.2),
        (
            "worker",
            "submit",
            json!([1, "{token1}", [{"artifact": "t1-tests", "revision": "R2"}]]),
            25.0,
        ),
        ("parent", "verify_task", json!([1, "{token1}", "R2"]), 25.1),
    ] {
        call(member, method, args, offset);
    }
    let claimed = call("worker", "claim", json!([2]), 26.0);
    let token2 = claimed["token"].as_str().expect("a claim token").to_owned();
    call(
        "worker",
        "submit",
        json!([2, token2, [{"artifact": "t2-docs", "revision": "R2"}]]),
        26.1,
    );
    call("parent", "verify_task", json!([2, token2, "R2"]), 26.2);
    // The evidence Python recorded at R1 is stale: completion needs R2.
    assert_eq!(
        board.call("parent", "complete", &json!(["R2"]), NOW + 27.0),
        Outcome::Refused(
            "completion requires accepted evidence at the current revision for every criterion"
                .to_owned()
        )
    );
    call(
        "parent",
        "evidence",
        json!(["tests", "final-check", "R2", "command", true]),
        28.0,
    );
    call(
        "parent",
        "evidence",
        json!(["review", "final-check", "R2", "review", true]),
        28.1,
    );
    call("parent", "complete", json!(["R2"]), 29.0);
    // The outcome is held for the supervisor, who makes it terminal.
    let held = call("supervisor", "_status", json!([]), 29.5);
    assert_eq!(held["outcome"], json!("succeeded"), "{held}");
    call("parent", "_close", json!([]), 30.0);
    let status = call("supervisor", "_status", json!([]), 31.0);
    assert_eq!(status["status"], json!("succeeded"), "{status}");
    assert_eq!(status["outcome"], json!("succeeded"), "{status}");
    assert_eq!(stored(&database, "PRAGMA integrity_check"), "ok");
    assert_eq!(stored(&database, "PRAGMA journal_mode"), "delete");
}
