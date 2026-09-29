//! Differential scenarios (#2270, epic #2265): run creation and run status
//! on the Python board and the Rust board, compared after every step by
//! result, refusal text and logical database dump.

#[path = "../common/swarm_board_diff/mod.rs"]
pub(crate) mod swarm_board_diff;

use rusqlite::Connection;
use serde_json::{Map, Value, json};
use swarm_board_diff::Outcome;
use swarm_board_diff::dump::{Dump, first_difference, logical_dump};
use swarm_board_diff::scenario::{Hold, held, run_both, sql, step, try_run_both};

pub(crate) const NOW: f64 = 1_700_000_000.25;
const HOUR: f64 = 3_600.0;
const DAY: f64 = 86_400.0;

fn criteria() -> Value {
    json!([
        {"id": "tests", "kind": "command", "description": "cargo test --test integration"},
        {"id": "review", "kind": "review", "description": "two cold reviews agree ✓"},
    ])
}

fn contract(deadline: Value) -> Value {
    json!({
        "goal": "ship the board — naïve café 😀",
        "constraints": ["no python", "keep the schema"],
        "criteria": criteria(),
        "member_limit": 5,
        "deadline": deadline,
    })
}

#[test]
fn bootstrap_then_status_is_identical() {
    run_both(&[
        step("supervisor", "_status", json!([]), NOW),
        step(
            "parent",
            "bootstrap_run",
            json!([4242, "Mon Sep 28 10:00:00 2026", "/run/parent.sock"]),
            NOW,
        ),
        step("supervisor", "_status", json!([]), NOW + 1.0),
        step("parent", "_snapshot", json!([]), NOW + 2.0),
        step("stranger", "_snapshot", json!([]), NOW + 3.0),
        step(
            "parent",
            "bootstrap_run",
            json!([1, "later", null]),
            NOW + 4.0,
        ),
        step("supervisor", "_status", json!({}), NOW + 5.0),
    ]);
}

#[test]
fn create_fresh_run_is_identical() {
    run_both(&[
        step("parent", "create_run", contract(json!(NOW + HOUR)), NOW),
        step("supervisor", "_status", json!([]), NOW + 1.0),
        step("parent", "_snapshot", json!([]), NOW + 2.0),
        // An integer deadline is stored as REAL and recorded as given.
        step("parent", "create_run", contract(json!(1_700_003_600)), NOW),
    ]);
    run_both(&[step(
        "parent",
        "create_run",
        contract(json!(1_700_003_600)),
        NOW,
    )]);
}

#[test]
fn create_over_setup_placeholder_is_identical() {
    run_both(&[
        step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
        step("worker", "create_run", contract(json!(NOW + HOUR)), NOW),
        step(
            "parent",
            "create_run",
            json!([
                "goal",
                ["one"],
                [{"id": "c", "kind": "review", "description": "d"}],
                1,
                NOW + DAY
            ]),
            NOW + 1.0,
        ),
        step("supervisor", "_status", json!([]), NOW + 2.0),
        step("parent", "_snapshot", json!([]), NOW + 3.0),
        step(
            "parent",
            "create_run",
            contract(json!(NOW + HOUR)),
            NOW + 4.0,
        ),
    ]);
}

#[test]
fn create_refusals_are_identical() {
    let with = |key: &str, value: Value| {
        let mut args = contract(json!(NOW + HOUR));
        args[key] = value;
        args
    };
    let mut duplicate = criteria();
    duplicate[1]["id"] = json!("tests");
    run_both(&[
        step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
        step("parent", "create_run", with("member_limit", json!(0)), NOW),
        step("parent", "create_run", with("member_limit", json!(26)), NOW),
        step(
            "parent",
            "create_run",
            with("member_limit", json!(true)),
            NOW,
        ),
        step(
            "parent",
            "create_run",
            with("member_limit", json!(3.0)),
            NOW,
        ),
        step(
            "parent",
            "create_run",
            with("deadline", json!(NOW - 1.0)),
            NOW,
        ),
        step("parent", "create_run", with("deadline", json!(NOW)), NOW),
        step(
            "parent",
            "create_run",
            with("deadline", json!(NOW + 8.0 * DAY)),
            NOW,
        ),
        step("parent", "create_run", with("criteria", duplicate), NOW),
        step(
            "parent",
            "create_run",
            with("goal", json!("g".repeat(8_193))),
            NOW,
        ),
        step("parent", "create_run", with("constraints", json!("x")), NOW),
        step("parent", "create_run", with("criteria", json!([])), NOW),
        step("supervisor", "_status", json!([]), NOW),
    ]);
}

#[test]
fn snapshot_after_expiry_is_identical() {
    run_both(&[
        step("parent", "create_run", contract(json!(NOW + 60.0)), NOW),
        step("parent", "_snapshot", json!([]), NOW + 10.0),
        // The deadline passes: the first transaction commits the pause.
        step("parent", "_snapshot", json!([]), NOW + 61.5),
        step("parent", "_snapshot", json!([]), NOW + 70.0),
        step("supervisor", "_status", json!([]), NOW + 80.0),
    ]);
}

/// A board another connection holds (#2303 review M1): a call waits out a
/// lock let go within the timeout, and one held past it is refused with
/// Python's text, reads and writes alike; the event-log run records each
/// as busy.
#[test]
fn a_busy_board_waits_or_refuses_as_python_does() {
    run_both(&[
        step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
        held(
            Hold::WaitedOut,
            step("supervisor", "_status", json!([]), NOW),
        ),
        held(
            Hold::Throughout,
            step("supervisor", "_status", json!([]), NOW),
        ),
        held(
            Hold::Throughout,
            step("parent", "create_run", contract(json!(NOW + HOUR)), NOW),
        ),
        held(
            Hold::WaitedOut,
            step("parent", "create_run", contract(json!(NOW + HOUR)), NOW),
        ),
        step("parent", "_snapshot", json!([]), NOW + 1.0),
    ]);
}

/// The comparator can fail: tampering with the Rust board after a step is
/// reported with the table and both rows.
#[test]
fn harness_self_test_detects_a_difference() {
    let steps = [
        step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
        step("supervisor", "_status", json!([]), NOW),
    ];
    let difference = try_run_both(&steps, |index, rust, _| {
        if index == 0 {
            Connection::open(rust)
                .unwrap()
                .execute("UPDATE events SET action='tampered'", [])
                .unwrap();
        }
    })
    .unwrap_err();
    assert!(
        difference.starts_with("step 0: bootstrap_run"),
        "{difference}"
    );
    assert!(difference.contains("events row 0"), "{difference}");
    assert!(difference.contains("tampered"), "{difference}");
    assert!(difference.contains("container_setup"), "{difference}");

    // A changed column is caught as well as a changed row.
    let difference = try_run_both(&steps, |index, rust, _| {
        if index == 0 {
            Connection::open(rust)
                .unwrap()
                .execute("UPDATE run SET status='running'", [])
                .unwrap();
        }
    })
    .unwrap_err();
    assert!(difference.contains("boards differ"), "{difference}");
}

/// #2283: the golden comparison can fail. A fixture recorded from the
/// board itself passes; the same fixture with one value changed, in a
/// copy (an answer, a step's file digest, a cell of the final dump), fails
/// the replay at the step it names.
#[test]
fn golden_replay_detects_a_changed_rust_result() {
    use swarm_board_diff::golden::fixture_path;
    use swarm_board_diff::scenario::{record_rust, try_run_golden_in};
    let steps = [
        step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
        step("supervisor", "_status", json!([]), NOW + 1.0),
        step("parent", "_snapshot", json!([]), NOW + 2.0),
    ];
    let dir = tempfile::tempdir().unwrap();
    record_rust(&steps).save(dir.path(), "self_test", &steps);
    assert_eq!(try_run_golden_in(dir.path(), "self_test", &steps), Ok(()));
    let path = fixture_path(dir.path(), "self_test", &steps);
    let recorded = std::fs::read_to_string(&path).unwrap();
    let corrupt = |change: &dyn Fn(&mut Value)| {
        let mut golden: Value = serde_json::from_str(&recorded).unwrap();
        change(&mut golden);
        std::fs::write(&path, serde_json::to_string(&golden).unwrap()).unwrap();
        try_run_golden_in(dir.path(), "self_test", &steps)
    };
    // An answer: `bootstrap_run` answered `null`.
    let difference = corrupt(&|golden| golden["answers"][0] = json!({"ok": "1"})).unwrap_err();
    assert!(
        difference.starts_with("step 0: bootstrap_run") && difference.contains("results differ"),
        "{difference}"
    );
    // A refusal's text.
    let difference =
        corrupt(&|golden| golden["answers"][1] = json!({"refused": "no"})).unwrap_err();
    assert!(
        difference.starts_with("step 1: _status") && difference.contains("results differ"),
        "{difference}"
    );
    // A step's board file.
    let difference = corrupt(&|golden| golden["files"][0] = json!("dump:0")).unwrap_err();
    assert!(
        difference.starts_with("step 0: bootstrap_run") && difference.contains("boards differ"),
        "{difference}"
    );
    // A cell of the final dump, shown row by row.
    let difference = corrupt(&|golden| {
        let text = golden["dump"]
            .to_string()
            .replacen("container_setup", "tampered", 1);
        golden["dump"] = serde_json::from_str(&text).unwrap();
        golden["files"][2] = json!("dump:0");
    })
    .unwrap_err();
    assert!(
        difference.starts_with("step 2: _snapshot") && difference.contains("tampered"),
        "{difference}"
    );
}

/// The result comparison can fail (#2270 review M1): a Rust answer that
/// differs while both files still match is reported as a result
/// difference, not passed over.
#[test]
fn harness_self_test_detects_a_result_difference() {
    let steps = [
        step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
        step("supervisor", "_status", json!([]), NOW),
    ];
    let difference = try_run_both(&steps, |index, _, rust| {
        if index == 0 {
            *rust = Outcome::Ok(json!(1));
        }
    })
    .unwrap_err();
    assert!(
        difference.starts_with("step 0: bootstrap_run"),
        "{difference}"
    );
    assert!(difference.contains("results differ"), "{difference}");
    // A refusal in place of an answer, and a changed refusal text, too.
    for tampered in [Outcome::Refused("no".to_owned()), Outcome::Ok(Value::Null)] {
        let difference = try_run_both(&steps, |index, _, rust| {
            if index == 1 {
                *rust = tampered.clone();
            }
        })
        .unwrap_err();
        assert!(difference.starts_with("step 1: _status"), "{difference}");
        assert!(difference.contains("results differ"), "{difference}");
    }
}

/// Key order is part of a result (#2270 review L4): Python's dicts keep
/// insertion order and the tool output shows it, so the same entries in
/// another order are a difference.
#[test]
fn harness_self_test_detects_a_key_order_difference() {
    let steps = [
        step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
        step("supervisor", "_status", json!([]), NOW),
    ];
    let mut reordered = false;
    let difference = try_run_both(&steps, |index, _, rust| {
        if let (1, Outcome::Ok(Value::Object(entries))) = (index, &*rust) {
            let reversed: Map<String, Value> = entries
                .iter()
                .rev()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            *rust = Outcome::Ok(Value::Object(reversed));
            reordered = true;
        }
    })
    .unwrap_err();
    assert!(reordered, "_status answered a dict to reorder");
    assert!(difference.starts_with("step 1: _status"), "{difference}");
    assert!(difference.contains("results differ"), "{difference}");
}

fn dump_of(statements: &str) -> (tempfile::TempDir, Dump) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("board.sqlite");
    Connection::open(&path)
        .unwrap()
        .execute_batch(statements)
        .unwrap();
    let dump = logical_dump(&path);
    (dir, dump)
}

/// Each storage class is its own value: an INTEGER never equals a REAL,
/// TEXT never a BLOB or an INTEGER, NULL never empty text, and a REAL is
/// its bits (`-0.0` is not `0.0`).
#[test]
fn the_comparator_compares_typed_values() {
    let table = "CREATE TABLE t (x);";
    let pairs = [
        ("300", "300.0"),
        ("'1'", "1"),
        ("x'31'", "'1'"),
        ("NULL", "''"),
        ("1.5", "'1.5'"),
        // A REAL compares by its bits: Python's `-0.0` is not `0.0`.
        ("0.0", "-0.0"),
    ];
    for (left, right) in pairs {
        let (_a, python) = dump_of(&format!("{table} INSERT INTO t VALUES({left});"));
        let (_b, rust) = dump_of(&format!("{table} INSERT INTO t VALUES({right});"));
        let difference = first_difference(&python, &rust);
        assert!(
            difference
                .as_deref()
                .is_some_and(|text| text.starts_with("t row 0")),
            "{left} vs {right}: {difference:?}"
        );
        let (_c, same) = dump_of(&format!("{table} INSERT INTO t VALUES({left});"));
        assert_eq!(first_difference(&python, &same), None, "{left}");
    }
}

#[test]
fn the_comparator_covers_the_schema_rows_counts_and_pragmas() {
    let base = "CREATE TABLE t (x); INSERT INTO t VALUES(1);";
    let (_a, python) = dump_of(base);
    for (change, expected) in [
        ("CREATE INDEX t_x ON t(x);", "sqlite_master"),
        ("INSERT INTO t VALUES(2);", "t: python holds 1 rows, rust 2"),
        ("PRAGMA user_version=3;", "PRAGMA user_version"),
        ("CREATE TABLE u (y);", "tables differ"),
    ] {
        let (_b, rust) = dump_of(&format!("{base} {change}"));
        let difference = first_difference(&python, &rust).unwrap_or_default();
        assert!(difference.contains(expected), "{change}: {difference}");
    }
    // `files` compares by path, whatever order the rows were written in.
    let files = "CREATE TABLE files (path TEXT PRIMARY KEY, task INTEGER);";
    let (_c, forward) = dump_of(&format!(
        "{files} INSERT INTO files VALUES('a',1); INSERT INTO files VALUES('b',2);"
    ));
    let (_d, backward) = dump_of(&format!(
        "{files} INSERT INTO files VALUES('b',2); INSERT INTO files VALUES('a',1);"
    ));
    assert_eq!(first_difference(&forward, &backward), None);
    // Any other table compares in rowid order.
    let (_e, first) =
        dump_of("CREATE TABLE t (x); INSERT INTO t VALUES('a'); INSERT INTO t VALUES('b');");
    let (_f, second) =
        dump_of("CREATE TABLE t (x); INSERT INTO t VALUES('b'); INSERT INTO t VALUES('a');");
    assert!(first_difference(&first, &second).is_some());
}

/// A NULL `run.status` (#2270 review L2) reads as Python reads it:
/// `_snapshot` answers `status: null`, `_status` answers it too, and
/// `create` over it is refused as over any run not in setup.
#[test]
fn a_null_run_status_reads_as_python_reads_it() {
    run_both(&[
        step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
        sql("UPDATE run SET status=NULL"),
        step("parent", "_snapshot", json!([]), NOW + 1.0),
        step("supervisor", "_status", json!([]), NOW + 2.0),
        step(
            "parent",
            "create_run",
            json!(["g", [], [{"id": "c", "kind": "review", "description": "d"}], 5, NOW + 3_600.0]),
            NOW + 3.0,
        ),
        sql("UPDATE run SET deadline=0.5"),
        step("parent", "_snapshot", json!([]), NOW + 4.0),
    ]);
}

/// A run with no row for `_status` to read, after one existed.
#[test]
fn status_after_the_run_row_is_deleted_is_identical() {
    run_both(&[
        step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
        sql("DELETE FROM run"),
        step("supervisor", "_status", json!([]), NOW + 1.0),
        step("parent", "_snapshot", json!([]), NOW + 2.0),
    ]);
}
