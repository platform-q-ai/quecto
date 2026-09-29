//! Differential scenarios for loosely typed values (#2270 review L3, L5,
//! L6; epic #2265 P3): arguments Python's `sqlite3` binds as it finds them,
//! and rows holding types the board never writes, read back through the
//! served methods. Each must match; where the Rust board deliberately
//! differs, the divergence is named in [`PERMITTED_DIVERGENCES`] and pinned
//! by the test of the same name.
use std::path::Path;

use serde_json::{Value, json};

use crate::swarm_board_diff_membership::{at, create};
use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::rust::RustBoard;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{
    Step, run_both, run_rust, sql, step, step_text, try_run_both,
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
///   within u64 (a `pid`, `started` or `socket`, a membership method's
///   member or reservation, #2271, or a task id, #2272) makes Python's
///   `sqlite3` raise `OverflowError` when it is bound, which is not an `sqlite3.Error`,
///   so the store does not turn it into a refusal and the call raises.
///   The Rust board refuses it as a store failure naming Python's
///   parameter position. (Compared before it is bound, such an integer
///   answers as Python's: it equals no stored value but an equal REAL.)
/// - `outside_edited_columns`: a `run` column `_snapshot` reads as a
///   number (`deadline`, `member_limit`) holding anything but the number
///   the board writes (NULL, text, a REAL `member_limit`), or a BLOB in
///   any column `_snapshot` or `_status` reads, or a number in a text
///   column they read (`status`, `coordinator`, `id` in a table rebuilt
///   without column types), is refused as a store failure; Python answers
///   with the value (or, for a BLOB, fails to write its JSON). Everything
///   else a file edited outside the board may
///   hold is read as Python reads it
///   (`outside_edited_text_reads_as_python_reads_it` and the tests before
///   it): text that is not UTF-8 in any column of a row Python fetches is
///   refused with Python's `Could not decode to UTF-8` text; `create`
///   fetches the whole run row and takes a status or coordinator that is
///   not text (a BLOB, or a number without TEXT affinity) as not the setup
///   placeholder;
///   `_bootstrap` reads no column; the join (#2271) reads the
///   coordinator column alone and takes one that is not text (a BLOB, or
///   a number without TEXT affinity) as nobody, where Python acts as that
///   value: the gate refuses both unless a member row's id is that same
///   value, when Python joins and Rust refuses (pinned below; matching it
///   would thread a non-text actor through every use case, a cost out of
///   proportion to a board no harness writes); a NULL run or member status, a NULL
///   coordinator, a NULL member id, added member columns and loosely typed
///   pids are read as they are stored.
/// - `outside_edited_task_columns` (#2272), values only a file edited
///   outside the board holds: a task's `acceptance`, `dependencies` or
///   `evidence` that is not JSON text is refused as a store failure where
///   Python's `json.loads` raises (`JSONDecodeError`, `TypeError`).
///   Stored dependencies are read as a list of task ids and anything else
///   as none, where Python iterates what it loaded: a dict's keys
///   (`{"2":1}` names task 2, so the task reads blocked and `claim`
///   refuses `unmet dependencies`; Rust reads it ready and claims it), a
///   text's characters (`"12"` names tasks 1 and 2), `null` or a number
///   (`TypeError` in `_task`), and a list entry (`[[2]]` raises
///   "unhashable" in the cycle check, where Rust keys it by its JSON text).
///   A stored dependency naming no task is incomplete: a ready task reads
///   blocked, where Python's `fetchone()[0]` raises `TypeError`; and
///   `claim` refuses `unmet dependencies`, where for a task that is not
///   ready (whose dependencies `_task` does not check) Python's claim
///   reads the missing dependency with `_task` and refuses `unknown task`.
///   For a completed dependency with invalid JSON in `acceptance`, Python's
///   `claim` loads the dependency's full `_task` and raises `JSONDecodeError`;
///   Rust reads only its status and claims the dependent task (pinned below).
/// - `outside_edited_evidence` (#2272): stored evidence that is not a list
///   of objects each carrying `revision` (only an edit holds it) meets
///   `verify_task` as a refusal, where Python raises or iterates the value.
/// - `outside_edited_contract` (#2273, listed case by case and pinned in
///   `swarm_board_diff_loose_completion.rs`): a run contract, a criterion
///   or a task's evidence only a file edited outside the board holds
///   meets `complete`, `evidence` and `amend` as a refusal (naming the
///   record, as a store failure, or as stale or unmatched evidence) where
///   Python raises or goes on.
/// - `outside_edited_control_records` (#2273, pinned in
///   `swarm_board_diff_loose_runs.rs` and `extend_run_deadline_tests`): a
///   pause record whose `started` is not a number (a boolean included), or
///   a usage budget that is not an object or whose limit is not a count,
///   is refused naming the record, and so are usage totals that are not
///   counts (a REAL or negative sum), but only where a paused run's
///   budget with a non-null token limit is checked for a resume
///   (elsewhere they pass through as Python passes them), where Python
///   raises or computes with them; an
///   integer `started` is read as its float, so an extension from it
///   records a float deadline where Python records the integer; and a loss
///   event whose detail is not an object, or whose `member` is not text
///   (an unhashable list or object included), names no member, where
///   Python raises.
/// - `real_to_text_digits` (#2269 review M1, pinned by
///   `swarm_board::binding_tests`): a float meeting a TEXT column is
///   written with the bundled SQLite's digits, which some hosts' libraries
///   (and so Python there) write with fewer.
/// - `unknown_member_status_is_not_alive` (owner decision in #2295, pinned
///   by `domain::swarm::policy_tests` and `policy_null_status_tests`): Python's `authorize` and
///   `admission` refuse only a member whose status is `'dead'`, so an
///   unknown or NULL status counts as alive there; Rust accepts only
///   `live` or `reserved` (an affirmative guard), so such a member may
///   read but not mutate, and is not an idempotent admission retry. The
///   membership methods (#2271) keep it: `_activate` and `_record_launch`
///   take such a member's reservation as stale, where Python goes on
///   (pinned by `activate_member_tests` and `record_member_launch_tests`).
pub const PERMITTED_DIVERGENCES: [&str; 9] = [
    "arguments_beyond_a_serde_value",
    "integer_beyond_i64_is_refused",
    "outside_edited_columns",
    "outside_edited_contract",
    "outside_edited_control_records",
    "outside_edited_evidence",
    "outside_edited_task_columns",
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

/// `create_run`'s arguments as JSON text, a criterion's extra key `w`
/// written as `extra`.
pub(crate) fn create_text(extra: &str) -> String {
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
    // The membership methods (#2271) and the task methods (#2272) bind
    // their arguments the same way.
    let bootstrap = json!([7, "s", null]);
    for (method, args, position) in [
        ("_socket", json!([u64::MAX]), 1),
        ("_release_unlaunched", json!([u64::MAX]), 1),
        // A task id (#2272), read under the read-only gate a setup run
        // passes.
        ("task_raw", json!([u64::MAX]), 1),
        // `bootstrap_run` draws the run's id, then the reservation.
        (
            "_activate",
            json!(["parent", format!("{:032x}", 2), 7, "s", u64::MAX]),
            3,
        ),
    ] {
        let difference = try_run_both(
            &[
                step("parent", "bootstrap_run", bootstrap.clone(), NOW),
                step("parent", method, args.clone(), NOW + 1.0),
            ],
            |_, _, _| {},
        )
        .unwrap_err();
        assert_eq!(
            difference,
            format!(
                "step 1: {method} as parent with {args} at {}: Python raised \
                 OverflowError: Python int too large to convert to SQLite INTEGER",
                NOW + 1.0
            )
        );
        let outcomes = rust_alone(&[
            ("parent", "bootstrap_run", bootstrap.clone()),
            ("parent", method, args),
        ]);
        assert_eq!(
            outcomes[1],
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
    // The join (#2271) reads the coordinator column alone and takes one
    // that is not text as nobody, which the gate refuses; Python acts as
    // the value it finds, which the gate finds when a member row holds
    // that same value as its id: a BLOB id (TEXT affinity keeps a BLOB),
    // or a number's text under TEXT affinity.
    for edit in [
        "UPDATE run SET coordinator=x'706172656e74';
         INSERT INTO members(id,reservation,status) VALUES(x'706172656e74','c','live')",
        "DROP TABLE run;
         CREATE TABLE run (id, goal, constraints, criteria, coordinator, integrator, member_limit, deadline, status);
         INSERT INTO run(id,status,coordinator,member_limit,deadline) VALUES('r','running',2.5,5,4e9);
         INSERT INTO members(id,reservation,status) VALUES('2.5','c','live')",
    ] {
        let difference = try_run_both(
            &[
                step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
                sql(edit),
                step("worker", "bootstrap_join", json!([8, "t", null]), NOW + 1.0),
            ],
            |_, _, _| {},
        )
        .unwrap_err();
        assert_eq!(
            difference,
            format!(
                "step 2: bootstrap_join as worker with [8,\"t\",null] at {}: results differ\n  \
                 python Ok(Null)\n  \
                 rust   {:?}",
                NOW + 1.0,
                Outcome::Refused("invoking member is unknown or death confirmed".into())
            ),
            "{edit}"
        );
    }
    // Without a member of that id, Python's gate refuses the join too.
    run_both(&[
        step("parent", "bootstrap_run", json!([7, "s", "/p.sock"]), NOW),
        sql("UPDATE run SET coordinator=x'706172656e74'"),
        step("worker", "bootstrap_join", json!([8, "t", null]), NOW + 1.0),
    ]);
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
const EXTERNAL_PINS: [(&str, &str, &str); 8] = [
    (
        "outside_edited_contract",
        include_str!("swarm_board_diff_loose_completion.rs"),
        "outside_edited_contract",
    ),
    (
        "outside_edited_control_records",
        include_str!("swarm_board_diff_loose_runs.rs"),
        "outside_edited_control_records",
    ),
    (
        "outside_edited_control_records",
        include_str!("../../src/application/swarm/use_cases/extend_run_deadline_tests.rs"),
        "a_pause_start_that_is_not_a_float_diverges",
    ),
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
    (
        "unknown_member_status_is_not_alive",
        include_str!("../../src/application/swarm/use_cases/activate_member_tests.rs"),
        "an_unknown_member_status_is_not_activated",
    ),
    (
        "unknown_member_status_is_not_alive",
        include_str!("../../src/application/swarm/use_cases/record_member_launch_tests.rs"),
        "an_unknown_member_status_records_no_launch",
    ),
];

/// `outside_edited_task_columns`: each hand edit of task 1 (after
/// `before`), then the `probe` where the boards differ: `python` is in
/// Python's side of the difference, `rust` in the Rust board's answer.
#[test]
fn outside_edited_task_columns() {
    let setup = [
        create(5),
        at(1.0, "parent", "_admit", json!(["w1", "r1"])),
        at(
            2.0,
            "parent",
            "_activate",
            json!(["w1", "r1", 11, "t", null]),
        ),
        at(3.0, "parent", "_admit", json!(["w2", "r2"])),
        at(
            4.0,
            "parent",
            "_activate",
            json!(["w2", "r2", 12, "t", null]),
        ),
        at(5.0, "w1", "task_create", json!(["a", "one", ["ok"]])),
        at(6.0, "w1", "task_create", json!(["b", "two", ["ok"]])),
    ];
    let pinned = |before: &[Step], edit: &str, probe: Step, python: &str, rust: &str| {
        let mut steps = setup.to_vec();
        steps.extend_from_slice(before);
        let prefix = format!("step {}: {}", steps.len() + 1, probe.method);
        steps.extend([sql(&format!("UPDATE tasks SET {edit} WHERE id=1")), probe]);
        let difference = try_run_both(&steps, |_, _, _| {}).unwrap_err();
        let python_side = difference.split("\n  rust").next().unwrap_or_default();
        assert!(
            difference.starts_with(&prefix) && python_side.contains(python),
            "{edit}: {difference}"
        );
        let outcome = format!("{:?}", run_rust(&steps));
        assert!(outcome.contains(rust), "{edit}: {outcome}");
    };
    let raw = || at(7.0, "parent", "task_raw", json!([1]));
    let claim = |member| at(8.0, member, "claim", json!([1]));
    let (ready, blocked) = (
        r#""status": String("ready")"#,
        r#""status": String("blocked")"#,
    );
    for (edit, python, rust) in [
        ("acceptance='not json'", "raised JSONDecodeError", CONTENDED),
        ("evidence=NULL", "raised TypeError", CONTENDED),
        ("dependencies='[99]'", "raised TypeError", blocked),
        (r#"dependencies='{"2":1}'"#, blocked, ready),
        (r#"dependencies='"12"'"#, blocked, ready),
        ("dependencies='null'", "raised TypeError", ready),
        ("dependencies='5'", "raised TypeError", ready),
    ] {
        pinned(&[], edit, raw(), python, rust);
    }
    let unknown = r#"python Refused("unknown task")"#;
    let unmet = r#"Refused("unmet dependencies")"#;
    pinned(
        &[claim("w1")],
        "dependencies='[99]'",
        claim("w2"),
        unknown,
        unmet,
    );
    // Claim checks the completed dependency using Python's full `_task`,
    // including its acceptance JSON; Rust reads only its status.
    pinned(
        &[
            at(7.0, "w1", "dependencies", json!([2, [1]])),
            sql("UPDATE tasks SET status='completed' WHERE id=1"),
        ],
        "acceptance='not json'",
        at(8.0, "w2", "claim", json!([2])),
        "raised JSONDecodeError",
        r#""status": String("claimed")"#,
    );
    let claimed = r#""status": String("claimed")"#;
    pinned(
        &[],
        r#"dependencies='{"2":1}'"#,
        claim("w1"),
        unmet,
        claimed,
    );
    // The cycle check walks task 1's stored `[[2]]` from a new edge to it.
    for (probe, rust) in [
        (
            at(9.0, "w1", "task_create", json!(["c", "three", ["ok"], [1]])),
            blocked,
        ),
        (at(9.0, "w1", "dependencies", json!([2, [1]])), "Ok("),
    ] {
        pinned(
            &[],
            "dependencies='[[2]]'",
            probe,
            "unhashable type: 'list'",
            rust,
        );
    }
}

/// `outside_edited_evidence`: Python raises, or verifies an empty dict or
/// text, where the Rust board refuses.
#[test]
fn outside_edited_evidence() {
    let task = at(1.0, "parent", "task_create", json!(["r", "t", ["ok"]]));
    let mut steps = vec![create(5), task];
    for (evidence, python) in [
        (r#"[{"artifact":"a"}]"#, "Python raised KeyError"),
        (r#"{"revision":"R1"}"#, "Python raised TypeError"),
        ("null", "Python raised TypeError"),
        ("{}", "python Ok(Null)"),
        (r#""""#, "python Ok(Null)"),
    ] {
        steps.truncate(2);
        let edit = format!("UPDATE tasks SET status='submitted',token='t',evidence='{evidence}'");
        steps.extend([
            sql(&edit),
            at(2.0, "parent", "verify_task", json!([1, "t", "R1"])),
        ]);
        let difference = try_run_both(&steps, |_, _, _| {}).unwrap_err();
        assert!(
            difference.starts_with("step 3: verify_task") && difference.contains(python),
            "{evidence}: {difference}"
        );
        let outcome = format!("{:?}", run_rust(&steps));
        assert!(outcome.contains("not a list of revisioned"), "{outcome}");
    }
}
