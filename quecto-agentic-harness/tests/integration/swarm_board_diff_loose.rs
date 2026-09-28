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
    Step, run_both, sql, step, step_text, try_run_both,
};

/// Every way the Rust board may differ from the Python board on the
/// methods this slice serves. Each name is a test below.
///
/// - `arguments_beyond_a_serde_value` (#2270 round-3 review L1): the
///   dispatcher takes a `serde_json::Value`, so argument text is parsed
///   with `py_json::decode` (`json.loads`) and converted, and a value no
///   `Value` holds is refused before any board call: a non-finite float
///   (`NaN`, `Infinity`, or a literal such as `1e400` that overflows), a
///   string holding a lone surrogate escape, an integer outside
///   i64 ∪ u64, or nesting deeper than `SERDE_MAX_DEPTH`. Python binds or stores each of
///   them (or, for an integer beyond i64, raises `OverflowError`). S13/S14
///   must parse member input with `py_json`, as the harness does; a
///   `PyJson` dispatcher would end this divergence.
/// - `integer_beyond_i64_is_refused`: an integer argument beyond i64 but
///   within u64 (a `pid`, `started` or `socket`) makes Python's `sqlite3`
///   raise `OverflowError`, which is not an `sqlite3.Error`, so the store
///   does not turn it into a refusal and the call raises. The Rust board
///   refuses it as a store failure naming Python's parameter position.
/// - `outside_edited_columns`: a `run` column `_snapshot` reads as a
///   number (`deadline`, `member_limit`) holding anything but the number
///   the board writes (NULL, text, a REAL `member_limit`), or a BLOB in
///   any column `_snapshot` or `_status` reads, is refused as a store
///   failure; Python answers with the value (or, for a BLOB, fails to
///   write its JSON). Everything else a file edited outside the board may
///   hold is read as Python reads it
///   (`outside_edited_text_reads_as_python_reads_it` and the tests before
///   it): text that is not UTF-8 in any column of a row Python fetches is
///   refused with Python's `Could not decode to UTF-8` text; `create`
///   fetches the whole run row and takes a status or coordinator that is
///   not text (a BLOB, or a number without TEXT affinity) as not the setup
///   placeholder;
///   `_bootstrap` reads no column; a NULL run or member status, a NULL
///   coordinator, a NULL member id, added member columns and loosely typed
///   pids are read as they are stored.
/// - `real_to_text_digits` (#2269 review M1, pinned by
///   `swarm_board::binding_tests`): a float meeting a TEXT column is
///   written with the bundled SQLite's digits, which some hosts' libraries
///   (and so Python there) write with fewer.
/// - `unknown_member_status_is_not_alive` (owner decision in #2295, pinned
///   by `domain::swarm::policy_tests` and `policy_null_status_tests`): Python's `authorize` and
///   `admission` refuse only a member whose status is `'dead'`, so an
///   unknown or NULL status counts as alive there; Rust accepts only
///   `live` or `reserved` (an affirmative guard), so such a member may
///   read but not mutate, and is not an idempotent admission retry. No
///   method this slice serves reaches it: `_snapshot` reads, and the
///   `bootstrap_run` driver alias skips `_join`.
pub const PERMITTED_DIVERGENCES: [&str; 5] = [
    "arguments_beyond_a_serde_value",
    "integer_beyond_i64_is_refused",
    "outside_edited_columns",
    "real_to_text_digits",
    "unknown_member_status_is_not_alive",
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

/// `create_run`'s arguments as JSON text, a criterion's extra key `w`
/// written as `extra`.
fn create_text(extra: &str) -> String {
    format!(
        r#"["g", [], [{{"id": "c", "kind": "review", "description": "d", "w": {extra}}}], 5, {}]"#,
        NOW + 3_600.0
    )
}

/// Argument text is parsed by each side (#2270 round-3 review L1), and
/// the Rust side parses it as `json.loads` does: `-0` is the integer 0
/// (not the float `-0.0`), an exponent makes a float, an integer within
/// u64 stays exact, and a repeated key keeps its first position with its
/// last value.
#[test]
fn argument_text_parses_as_python_parses_it() {
    for extra in [
        "-0",
        "-0.0",
        "1E2",
        "1e-7",
        "12345678901234567890",
        r#"{"k": 1, "j": 2, "k": 3}"#,
        r#""\u00e9\ud83d\ude00""#,
    ] {
        run_both(&[
            step_text("parent", "create_run", &create_text(extra), NOW),
            step("parent", "_snapshot", json!([]), NOW + 1.0),
        ]);
    }
    run_both(&[
        step_text("parent", "bootstrap_run", r#"[-0, "s", -0.0]"#, NOW),
        step("parent", "_snapshot", json!([]), NOW + 1.0),
    ]);
}

/// Python stores each of these in the run's criteria; the Rust side
/// refuses the text before any board call, naming what no `Value` holds.
#[test]
fn arguments_beyond_a_serde_value() {
    let deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
    for (extra, reason) in [
        ("1e400", "Infinity has no JSON number form"),
        ("-Infinity", "-Infinity has no JSON number form"),
        ("NaN", "NaN has no JSON number form"),
        (r#""\ud800""#, r#""\ud800" holds a lone surrogate"#),
        (
            "18446744073709551616",
            "integer 18446744073709551616 is outside i64 and u64",
        ),
        (
            "-9223372036854775809",
            "integer -9223372036854775809 is outside i64 and u64",
        ),
        (&deep, "nesting deeper than 128 levels"),
    ] {
        let text = create_text(extra);
        let difference = try_run_both(
            &[step_text("parent", "create_run", &text, NOW)],
            |_, _, _| {},
        )
        .unwrap_err();
        assert_eq!(
            difference,
            format!(
                "step 0: create_run as parent with {text} at {NOW}: results differ\n  \
                 python Ok(Null)\n  \
                 rust   {:?}",
                Outcome::Refused(format!("{UNREPRESENTABLE}{reason}"))
            ),
            "{extra}"
        );
    }
}

/// `_bootstrap` asks only whether a run exists (`SELECT 1 FROM run`) and
/// `create` reads only the run's status and coordinator (#2270 round-3
/// review N1): columns either leaves unread may hold anything.
#[test]
fn bootstrap_and_create_read_only_the_run_columns_python_reads() {
    for edit in [
        "UPDATE run SET deadline='soon', member_limit='many'",
        "UPDATE run SET deadline=NULL, member_limit=2.5, integrator=x'00'",
    ] {
        run_both(&[
            step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
            sql(edit),
            step("parent", "bootstrap_run", json!([8, "t", null]), NOW + 1.0),
            step_text("parent", "create_run", &create_text("1"), NOW + 2.0),
            step("supervisor", "_status", json!([]), NOW + 3.0),
        ]);
    }
}

/// A snapshot's member rows are `dict(row)` (#2270 round-3 review N5):
/// every column the table has, in table order, each as stored, a NULL id
/// included.
#[test]
fn snapshot_member_rows_are_every_column_as_stored() {
    run_both(&[
        step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
        sql("ALTER TABLE members ADD COLUMN extra TEXT"),
        sql("ALTER TABLE members ADD COLUMN score REAL"),
        sql("UPDATE members SET extra='x', score=1.5"),
        sql("INSERT INTO members(id,status,started) VALUES(NULL,'live',3)"),
        step("parent", "_snapshot", json!([]), NOW + 1.0),
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

/// How the Rust side refuses argument text no `Value` holds.
const UNREPRESENTABLE: &str = "arguments: not representable as a serde_json value: ";

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

/// Text that is not UTF-8 in any column of a row Python fetches is refused
/// with Python's text (#2270 round-4 review L1, L3), its bytes shown with
/// one U+FFFD each: a run's status, coordinator or goal under `create`
/// (which fetches the whole row), `_snapshot` and `_status`, and a
/// member's `started` under `_snapshot`. A status or coordinator that is
/// not text (a BLOB, or a number in a table rebuilt without TEXT affinity)
/// is not the setup placeholder, so `create` refuses to reset it; a number
/// the TEXT affinity stores as its text is not the placeholder either.
#[test]
fn outside_edited_text_reads_as_python_reads_it() {
    let create = |now| step_text("parent", "create_run", &create_text("1"), now);
    for edit in [
        "UPDATE run SET status=CAST(x'ff' AS TEXT)",
        "UPDATE run SET coordinator=CAST(x'706172e282ff656e74' AS TEXT)",
        "UPDATE run SET goal=CAST(x'ff' AS TEXT)",
        "UPDATE members SET started=CAST(x'31ff32' AS TEXT)",
    ] {
        run_both(&[
            step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
            sql(edit),
            create(NOW + 1.0),
            step("parent", "_snapshot", json!([]), NOW + 2.0),
            step("supervisor", "_status", json!([]), NOW + 3.0),
        ]);
    }
    for edit in [
        "UPDATE run SET status=x'7365747570'",
        "UPDATE run SET status=1",
        "UPDATE run SET coordinator=x'706172656e74'",
        "UPDATE run SET coordinator=2.5",
        "DROP TABLE run;
         CREATE TABLE run (id, goal, constraints, criteria, coordinator, integrator, member_limit, deadline, status);
         INSERT INTO run(id,status,coordinator) VALUES('r',1,2.5)",
    ] {
        run_both(&[
            step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
            sql(edit),
            create(NOW + 1.0),
        ]);
    }
}

/// Each named divergence has its test in this file, and each test here
/// that pins one is named.
#[test]
fn every_permitted_divergence_is_pinned_by_name() {
    let source = include_str!("swarm_board_diff_loose.rs");
    for name in PERMITTED_DIVERGENCES {
        let pinned_here = source.contains(&format!("fn {name}()"));
        let pinned_elsewhere = EXTERNAL_PINS
            .iter()
            .filter(|(pinned, _, _)| *pinned == name)
            .map(|(_, file, test)| file.contains(&format!("fn {test}()")))
            .collect::<Vec<_>>();
        assert!(
            pinned_here
                || (!pinned_elsewhere.is_empty() && pinned_elsewhere.iter().all(|&found| found)),
            "{name} has no pinning test"
        );
    }
    for (name, _, test) in EXTERNAL_PINS {
        assert!(
            PERMITTED_DIVERGENCES.contains(&name),
            "{test} pins {name}, which is not a permitted divergence"
        );
    }
}

/// Divergences pinned outside this suite: the name, the test file's
/// source and the pinning test in it.
const EXTERNAL_PINS: [(&str, &str, &str); 3] = [
    (
        "real_to_text_digits",
        include_str!("../../src/infrastructure/persistence/swarm_board/binding_tests.rs"),
        "a_float_reaching_text_affinity_reads_as_the_bundled_sqlite_writes_it",
    ),
    (
        "unknown_member_status_is_not_alive",
        include_str!("../../src/domain/swarm/policy_tests.rs"),
        "unknown_statuses_found_in_a_file_are_refused_affirmatively",
    ),
    (
        "unknown_member_status_is_not_alive",
        include_str!("../../src/domain/swarm/policy_null_status_tests.rs"),
        "a_null_member_status_is_not_alive",
    ),
];
