//! Differential scenarios for loosely typed values (#2270 review L3, L5,
//! L6; epic #2265 P3): arguments Python's `sqlite3` binds as it finds them,
//! and rows holding types the board never writes, read back through the
//! served methods. Each must match; where the Rust board deliberately
//! differs, the divergence is named in [`PERMITTED_DIVERGENCES`] and pinned
//! by the test of the same name.
use std::path::Path;

use serde_json::{Value, json};

use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::rust::RustBoard;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{
    Step, run_both, sql, step, try_run_both,
};

/// Every way the Rust board may differ from the Python board on the
/// methods this slice serves. Each name is a test below.
///
/// - `integer_beyond_i64_is_refused`: an integer argument beyond i64 but
///   within u64 (a `pid`, `started` or `socket`) makes Python's `sqlite3`
///   raise `OverflowError`, which is not an `sqlite3.Error`, so the store
///   does not turn it into a refusal and the call raises. The Rust board
///   refuses it as a store failure naming Python's parameter position.
/// - `integer_beyond_u64_is_a_float`: the tool's JSON arguments are parsed
///   without arbitrary precision, so an integer beyond u64 reaches the
///   board as the nearest float. As a `bootstrap_run` argument it binds as
///   REAL, where Python raises `OverflowError`; inside `create_run`'s
///   criteria (an extra key of a criterion) it is stored, and recorded in
///   the `created` event's detail, as the float's text
///   (`1.8446744073709552e+19`), where Python keeps the integer's digits.
///   The fix belongs where arguments are parsed (S13/S14).
/// - `outside_edited_columns`: a `run` column a membership operation reads
///   as a number (`deadline`, `member_limit`) holding anything but the
///   number the board writes (NULL, text, a REAL `member_limit`), or a BLOB
///   in any column read, is refused as a store failure; Python answers with
///   the value (or, for a BLOB, fails to write its JSON). A NULL run or
///   member status, a NULL coordinator and loosely typed pids are read as
///   Python reads them.
/// - `real_to_text_digits` (#2269 review M1, pinned by
///   `swarm_board::binding_tests`): a float meeting a TEXT column is
///   written with the bundled SQLite's digits, which some hosts' libraries
///   (and so Python there) write with fewer.
pub const PERMITTED_DIVERGENCES: [&str; 4] = [
    "integer_beyond_i64_is_refused",
    "integer_beyond_u64_is_a_float",
    "outside_edited_columns",
    "real_to_text_digits",
];

/// `bootstrap_run` with `args` on a fresh board, then the snapshot that
/// shows the member row it wrote.
fn bootstrap_then_snapshot(args: Value) -> Vec<Step> {
    vec![
        step("parent", "bootstrap_run", args, NOW),
        step("parent", "_snapshot", json!([]), NOW + 1.0),
        step("supervisor", "_status", json!([]), NOW + 2.0),
    ]
}

/// P3: `True` binds as 1, and the column's affinity decides the rest: an
/// INTEGER `pid` keeps a non-integral float or non-numeric text as given
/// and converts numeric text; TEXT `started` and `socket` store a number's
/// text.
#[test]
fn bootstrap_binds_loose_arguments_as_python_does() {
    for args in [
        json!([true, 5, null]),
        json!([false, 5.5, true]),
        json!(["12", "s", 3]),
        json!([" 12 ", -7, "x"]),
        json!([3.5, null, null]),
        json!([3.0, null, -0.0]),
        json!(["3.0", "s", "/p.sock"]),
        json!(["abc", "", ""]),
        json!({"pid": true, "started": 5, "socket": null}),
        json!({"socket": 1.25, "started": false, "pid": -1}),
    ] {
        run_both(&bootstrap_then_snapshot(args));
    }
}

/// A list or an object is refused by both boards with the same text:
/// Python's `sqlite3` numbers the parameter by `_bootstrap`'s statement
/// (`VALUES(?,?,'live',?,?,?)`: `pid` 3, `started` 4, `socket` 5).
#[test]
fn unbindable_arguments_are_refused_as_python_refuses_them() {
    for (args, expected) in [
        (
            json!([[1], "s", null]),
            "Error binding parameter 3: type 'list' is not supported",
        ),
        (
            json!([5, [1], null]),
            "Error binding parameter 4: type 'list' is not supported",
        ),
        (
            json!([5, "s", {}]),
            "Error binding parameter 5: type 'dict' is not supported",
        ),
        (
            json!({"pid": 5, "started": "s", "socket": {"a": 1}}),
            "Error binding parameter 5: type 'dict' is not supported",
        ),
    ] {
        run_both(&bootstrap_then_snapshot(args.clone()));
        let outcomes = rust_alone(&[("parent", "bootstrap_run", args)]);
        assert_eq!(
            outcomes[0],
            Outcome::Refused(format!("{CONTENDED}{expected}"))
        );
    }
}

/// Rows the board never writes itself, read as Python reads them: a NULL
/// member status, a NULL coordinator, a pid of any storage class, and
/// `_status` reading only the columns Python's `_status` selects.
#[test]
fn loosely_typed_rows_read_as_python_reads_them() {
    run_both(&[
        step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
        sql("INSERT INTO members(id,status) VALUES('odd',NULL)"),
        sql("INSERT INTO members(id,status,pid) VALUES('text','live','abc')"),
        sql("INSERT INTO members(id,status,pid) VALUES('real','reserved',2.5)"),
        step("parent", "_snapshot", json!([]), NOW + 1.0),
        step("odd", "_snapshot", json!([]), NOW + 2.0),
        step("supervisor", "_status", json!([]), NOW + 3.0),
        sql("UPDATE members SET status=NULL WHERE id='parent'"),
        step("parent", "_snapshot", json!([]), NOW + 4.0),
        sql("UPDATE run SET coordinator=NULL"),
        step("parent", "_snapshot", json!([]), NOW + 5.0),
        step("supervisor", "_status", json!([]), NOW + 6.0),
        // `create` over the placeholder needs its coordinator.
        step(
            "parent",
            "create_run",
            json!(["g", [], [{"id": "c", "kind": "review", "description": "d"}], 5, NOW + 3_600.0]),
            NOW + 7.0,
        ),
    ]);
    run_both(&[
        step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
        // Columns `_status` does not select, and a NULL status it does.
        sql("UPDATE run SET member_limit='many', goal=x'00', criteria=NULL, integrator=3"),
        step("supervisor", "_status", json!([]), NOW + 1.0),
        sql("UPDATE run SET status=NULL, outcome=4"),
        step("supervisor", "_status", json!([]), NOW + 2.0),
    ]);
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

/// A board of its own, driven through the Rust side alone: for inputs the
/// Python side cannot answer as JSON.
fn rust_alone(steps: &[(&str, &str, Value)]) -> Vec<Outcome> {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("swarm.sqlite");
    let board = RustBoard::open(&database, dir.path());
    let outcomes = steps
        .iter()
        .map(|(member, method, args)| board.call(member, method, args, NOW))
        .collect();
    drop(board);
    assert_board_file(&database);
    outcomes
}

fn assert_board_file(database: &Path) {
    assert!(database.exists(), "{} was created", database.display());
}

fn refused_with(outcome: &Outcome, prefix: &str) -> bool {
    matches!(outcome, Outcome::Refused(text) if text.starts_with(prefix))
}

const CONTENDED: &str = "coordination store unavailable or contended: ";

#[test]
fn integer_beyond_i64_is_refused() {
    // An integer beyond i64 but within u64: Python raises OverflowError
    // (not a board refusal); Rust refuses it, naming Python's position.
    for (args, position) in [
        (json!([u64::MAX, "s", null]), 3),
        (json!([5, u64::MAX, null]), 4),
        (json!([5, "s", u64::MAX]), 5),
    ] {
        let difference = try_run_both(
            &[step("parent", "bootstrap_run", args.clone(), NOW)],
            |_, _, _| {},
        )
        .unwrap_err();
        assert_eq!(
            difference,
            format!(
                "step 0: bootstrap_run as parent with {args} at {NOW}: Python raised \
                 OverflowError: Python int too large to convert to SQLite INTEGER"
            )
        );
        let outcomes = rust_alone(&[("parent", "bootstrap_run", args)]);
        assert_eq!(
            outcomes[0],
            Outcome::Refused(format!(
                "{CONTENDED}Error binding parameter {position}: \
                 Python int too large to convert to SQLite INTEGER"
            ))
        );
    }
}

#[test]
fn integer_beyond_u64_is_a_float() {
    // serde_json, without arbitrary precision, parses 2**64 as a float.
    let args: Value = serde_json::from_str(r#"[18446744073709551616, "s", null]"#).unwrap();
    assert!(args[0].is_f64(), "{args}");
    let outcomes = rust_alone(&[
        ("parent", "bootstrap_run", args),
        ("parent", "_snapshot", json!([])),
    ]);
    assert_eq!(outcomes[0], Outcome::Ok(Value::Null));
    let Outcome::Ok(snapshot) = &outcomes[1] else {
        panic!("{:?}", outcomes[1]);
    };
    assert_eq!(
        serde_json::to_string(&snapshot["members"][0]["pid"]).unwrap(),
        "1.8446744073709552e+19"
    );
    // Inside `create_run`'s criteria, and so in the `created` event's
    // detail, the float's text is stored; Python stores
    // `"x":18446744073709551616`.
    let args: Value = serde_json::from_str(&format!(
        r#"["g", [], [{{"id": "c", "kind": "review", "description": "d", "x": 18446744073709551616}}], 5, {}]"#,
        NOW + 3_600.0
    ))
    .unwrap();
    assert!(args[2][0]["x"].is_f64(), "{args}");
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("swarm.sqlite");
    let board = RustBoard::open(&database, dir.path());
    assert_eq!(
        board.call("parent", "create_run", &args, NOW),
        Outcome::Ok(Value::Null)
    );
    drop(board);
    let connection = rusqlite::Connection::open(&database).unwrap();
    let criteria: String = connection
        .query_row("SELECT criteria FROM run", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        criteria,
        r#"[{"description":"d","id":"c","kind":"review","x":1.8446744073709552e+19}]"#
    );
    let detail: String = connection
        .query_row(
            "SELECT detail FROM events WHERE action='created'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        detail.contains(r#""criteria":[{"description":"d","id":"c","kind":"review","x":1.8446744073709552e+19}]"#),
        "{detail}"
    );
}

#[test]
fn outside_edited_columns() {
    for edit in [
        "UPDATE run SET deadline=NULL",
        "UPDATE run SET deadline='soon'",
        "UPDATE run SET member_limit='many'",
    ] {
        let difference = try_run_both(
            &[
                step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
                sql(edit),
                step("parent", "_snapshot", json!([]), NOW + 1.0),
            ],
            |_, _, _| {},
        )
        .unwrap_err();
        assert!(
            difference.starts_with("step 2: _snapshot") && difference.contains("results differ"),
            "{edit}: {difference}"
        );
        assert!(
            difference.contains("python Ok(")
                && difference.contains(&format!("rust   Refused(\"{CONTENDED}")),
            "{edit}: {difference}"
        );
    }
    // A BLOB: Python's answer is not JSON at all; Rust refuses it.
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("swarm.sqlite");
    let board = RustBoard::open(&database, dir.path());
    assert_eq!(
        board.call("parent", "bootstrap_run", &json!([7, "s", null]), NOW),
        Outcome::Ok(Value::Null)
    );
    rusqlite::Connection::open(&database)
        .unwrap()
        .execute("UPDATE members SET started=x'00'", [])
        .unwrap();
    let outcome = board.call("parent", "_snapshot", &json!([]), NOW);
    assert!(refused_with(&outcome, CONTENDED), "{outcome:?}");
}

/// Each named divergence has its test in this file, and each test here
/// that pins one is named.
#[test]
fn every_permitted_divergence_is_pinned_by_name() {
    let source = include_str!("swarm_board_diff_loose.rs");
    for name in PERMITTED_DIVERGENCES {
        let pinned = source.contains(&format!("fn {name}()"))
            || (name == "real_to_text_digits"
                && include_str!(
                    "../../src/infrastructure/persistence/swarm_board/binding_tests.rs"
                )
                .contains(
                    "fn a_float_reaching_text_affinity_reads_as_the_bundled_sqlite_writes_it()",
                ));
        assert!(pinned, "{name} has no pinning test");
    }
}
