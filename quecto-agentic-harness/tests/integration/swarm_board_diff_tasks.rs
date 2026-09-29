//! Differential scenarios (#2272, epic #2265): task creation, dependencies,
//! claims and releases on the Python board and the Rust board, compared
//! after every step by result, refusal text and logical database dump.
//! Ported from `tests/swarm_helpers_test.py`, plus the loosely typed
//! arguments Python accepts (epic P3).
use serde_json::{Value, json};

use crate::swarm_board_diff_membership::{at, create};
use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::Outcome;
use crate::swarm_board_diff_runs::swarm_board_diff::python::PyBoard;
use crate::swarm_board_diff_runs::swarm_board_diff::rust::RustBoard;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{Step, run_both, sql};

/// A running run of `limit` coordinated by `parent`, with `worker` live.
fn staffed(limit: i64) -> Vec<Step> {
    vec![
        create(limit),
        at(1.0, "parent", "_admit", json!(["worker", "res-w"])),
        at(
            2.0,
            "parent",
            "_activate",
            json!(["worker", "res-w", 11, "t", null]),
        ),
    ]
}

fn task(offset: f64, request: &str, title: &str, dependencies: Value) -> Step {
    at(
        offset,
        "worker",
        "task_create",
        json!([request, title, ["tests pass"], dependencies]),
    )
}

fn raw(offset: f64, task_id: Value) -> Step {
    at(offset, "parent", "task_raw", json!([task_id]))
}

/// The `n`th id the harness draws, as the counter writes it.
fn token(n: u64) -> String {
    format!("{n:032x}")
}

fn scenario(mut steps: Vec<Step>, more: impl IntoIterator<Item = Step>) -> Vec<Step> {
    steps.extend(more);
    steps
}

/// SQLite assigns a negative rowid after the highest hand-edited id is
/// negative; the board must still create and read that task in debug builds.
#[test]
fn hand_edited_negative_task_ids_allow_creation() {
    for (edited, inserted) in [(-5, -4), (-1, 0)] {
        run_both(&scenario(
            staffed(5),
            [
                task(3.0, "first", "first", json!(null)),
                sql(&format!("UPDATE tasks SET id={edited} WHERE id=1")),
                task(4.0, "next", "next", json!(null)),
                raw(5.0, json!(inserted)),
            ],
        ));
    }
}

/// A rebuilt table without its primary key can contain duplicate ids.
/// Python updates both matching rows; the Rust adapter must not panic.
#[test]
fn duplicate_task_ids_without_primary_key_allow_updates() {
    run_both(&scenario(
        staffed(5),
        [
            task(3.0, "first", "first", json!(null)),
            sql("ALTER TABLE tasks RENAME TO old_tasks;
                 CREATE TABLE tasks AS SELECT * FROM old_tasks;
                 DROP TABLE old_tasks;
                 INSERT INTO tasks SELECT * FROM tasks WHERE id=1"),
            at(4.0, "worker", "claim", json!([1])),
            raw(5.0, json!(1)),
        ],
    ));
}

/// `test_claims_are_atomic_and_dependencies_block_claims`, sequentially:
/// a dependent task reads blocked and is refused, the first claim wins and
/// every later one is refused; the claimed task carries its token.
#[test]
fn claims_are_atomic_and_dependencies_block_claims() {
    run_both(&scenario(
        staffed(5),
        [
            task(3.0, "task", "work", json!(null)),
            task(4.0, "dependent", "later", json!([1])),
            raw(5.0, json!(2)),
            at(6.0, "worker", "claim", json!([2])),
            at(7.0, "worker", "claim", json!({"task_id": 1})),
            at(8.0, "worker", "claim", json!([1])),
            at(9.0, "parent", "claim", json!([1])),
            raw(10.0, json!(1)),
            raw(11.0, json!(2)),
            at(12.0, "worker", "claim", json!([99])),
        ],
    ));
}

/// `test_request_retries_are_idempotent_but_payload_conflicts_fail`: the
/// replay answers the stored result (keys sorted), `None` and `[]` are one
/// payload, another payload under the id is refused, and ids are per
/// member.
#[test]
fn request_retries_are_idempotent_but_payload_conflicts_fail() {
    run_both(&scenario(
        staffed(5),
        [
            task(3.0, "task", "work", json!(null)),
            task(4.0, "task", "work", json!([])),
            at(
                5.0,
                "worker",
                "task_create",
                json!({"request": "task", "title": "work", "acceptance": ["tests pass"]}),
            ),
            at(
                6.0,
                "worker",
                "task_create",
                json!(["task", "different", ["pass"], []]),
            ),
            at(
                7.0,
                "parent",
                "task_create",
                json!(["task", "different", ["pass"], []]),
            ),
            // Replays answer even what the board holds now: task 1 claimed.
            at(8.0, "worker", "claim", json!([1])),
            task(9.0, "task", "work", json!(0)),
        ],
    ));
}

/// `test_task_acceptance_errors_teach_the_required_type`, the title and
/// request id bounds, and the order they are checked in.
#[test]
fn task_arguments_are_refused_identically() {
    let mut steps = staffed(5);
    let mut offset = 3.0;
    for args in [
        json!(["invalid", "work", "tests pass"]),
        json!(["invalid", "work", []]),
        json!(["invalid", "work", [42]]),
        json!(["invalid", "work", [""]]),
        json!(["invalid", "work", ["\u{3000}"]]),
        json!(["invalid", "work", {"a": "b"}]),
        json!(["invalid", "", ["ok"]]),
        json!(["invalid", " \n", ["ok"]]),
        json!(["invalid", 5, ["ok"]]),
        json!(["invalid", "t".repeat(1025), ["ok"]]),
        json!(["invalid", "é".repeat(512), ["ok"]]),
        json!(["invalid", "work", ["é".repeat(1366)]]),
        json!(["invalid", "work", ["é".repeat(1364)]]),
        json!([5, "work", ["ok"]]),
        json!(["", "work", ["ok"]]),
        json!(["r".repeat(129), "work", ["ok"]]),
        json!(["r".repeat(128), "work", ["ok"]]),
        json!([null, "work", ["ok"]]),
    ] {
        steps.push(at(offset, "worker", "task_create", args.clone()));
        // A stranger meets the argument checks before the gate.
        steps.push(at(offset + 0.5, "stranger", "task_create", args));
        offset += 1.0;
    }
    run_both(&steps);
}

/// `dependencies or []`: falsy values are no dependencies (one payload);
/// a truthy value that is not a list is inserted and then refused, which
/// rolls the insert back.
#[test]
fn dependency_arguments_are_normalized_and_checked_identically() {
    let mut steps = staffed(5);
    steps.push(task(3.0, "base", "base", json!(null)));
    let mut offset = 4.0;
    for (request, dependencies) in [
        ("zero", json!(0)),
        ("false", json!(false)),
        ("empty", json!("")),
        ("dict", json!({})),
        ("float", json!(0.0)),
        ("text", json!("abc")),
        ("object", json!({"a": 1})),
        ("number", json!(5)),
        ("true", json!([true])),
        ("float-id", json!([1.0])),
        ("string-id", json!(["1"])),
        ("missing", json!([99])),
        ("many", json!(vec![1; 101])),
        ("most", json!(vec![1; 100])),
        ("big", json!([u64::MAX])),
    ] {
        steps.push(task(offset, request, request, dependencies));
        offset += 1.0;
    }
    steps.push(raw(offset, json!(2)));
    run_both(&steps);
}

/// `test_dependencies_reject_missing_self_and_cycles`, and a loose task id:
/// `"1"` finds task 1 but is not equal to the dependency `1`, so Python
/// stores a self edge, which then blocks the task forever; `1.0` and
/// `true` equal it and are refused.
#[test]
fn dependencies_reject_missing_self_and_cycles() {
    run_both(&scenario(
        staffed(5),
        [
            task(3.0, "first", "first", json!(null)),
            task(4.0, "second", "second", json!([1])),
            task(5.0, "third", "third", json!(null)),
            at(6.0, "worker", "dependencies", json!([1, [99_999]])),
            at(7.0, "worker", "dependencies", json!([1, [1]])),
            at(8.0, "worker", "dependencies", json!([1, [2]])),
            at(
                9.0,
                "worker",
                "dependencies",
                json!({"task_id": 1, "dependencies": null}),
            ),
            at(10.0, "worker", "dependencies", json!([1.0, [1]])),
            at(11.0, "worker", "dependencies", json!([true, [2]])),
            at(12.0, "worker", "dependencies", json!([2, [3, 3]])),
            raw(13.0, json!(2)),
            at(14.0, "worker", "dependencies", json!(["1", [1]])),
            raw(15.0, json!(1)),
            at(16.0, "worker", "claim", json!([1])),
            at(17.0, "worker", "claim", json!([3])),
            at(18.0, "worker", "dependencies", json!([3, []])),
            at(19.0, "worker", "dependencies", json!([99, []])),
            at(20.0, "worker", "dependencies", json!([[1], []])),
        ],
    ));
}

/// P3: a task id bound as Python binds it. Numeric text, an integral float
/// and `true` find task 1 through INTEGER affinity and are written into
/// the events as given; other text and NULL find nothing; a list is
/// refused as Python's `sqlite3` refuses it.
#[test]
fn claim_with_a_string_task_id_matches_by_affinity_identically() {
    let mut steps = scenario(
        staffed(5),
        [
            task(3.0, "first", "first", json!(null)),
            task(4.0, "second", "second", json!(null)),
        ],
    );
    let mut offset = 5.0;
    for task_id in [
        json!("1"),
        json!(" 1 "),
        json!("1.0"),
        json!(1.0),
        json!(true),
        json!("one"),
        json!(null),
        json!(1.5),
        json!([1]),
        json!({"id": 1}),
    ] {
        steps.push(at(offset, "worker", "claim", json!([task_id.clone()])));
        steps.push(raw(offset + 0.25, task_id.clone()));
        steps.push(sql(
            "UPDATE tasks SET status='ready', owner=NULL, token=NULL WHERE id=1",
        ));
        offset += 1.0;
    }
    steps.push(at(offset, "worker", "claim", json!([true])));
    steps.push(sql("UPDATE tasks SET token='tok' WHERE id=1"));
    steps.push(at(offset + 1.0, "worker", "release", json!(["1.0", "tok"])));
    steps.push(raw(offset + 2.0, json!(1)));
    run_both(&steps);
}

/// `test_stale_claim_cannot_modify_recovered_work`'s claim-only prefix and
/// the release: a stale token, another member and unclaimed work are
/// refused; the owner's release reopens the task and deletes the files
/// reserved under that claim only.
#[test]
fn only_the_current_claim_releases_and_its_files_go() {
    run_both(&scenario(
        staffed(5),
        [
            task(3.0, "first", "first", json!(null)),
            task(4.0, "second", "second", json!(null)),
            at(5.0, "worker", "claim", json!([1])),
            at(6.0, "parent", "claim", json!([2])),
            sql(
                "INSERT INTO files SELECT 'a', 1, 'worker', token, 'r1' FROM tasks WHERE id=1;
                 INSERT INTO files VALUES('b', 1, 'worker', 'older', 'r2');
                 INSERT INTO files SELECT 'c', 2, 'parent', token, 'r3' FROM tasks WHERE id=2;",
            ),
            at(7.0, "worker", "release", json!([1, "stale"])),
            at(8.0, "worker", "release", json!([1, null])),
            at(
                9.0,
                "worker",
                "release",
                json!({"task_id": 2, "token": "stale"}),
            ),
            // `create` drew ids 1 and 2: the claims' tokens are 3 and 4.
            at(10.0, "parent", "release", json!([1, token(3)])),
            at(11.0, "worker", "release", json!([1, token(3)])),
            at(12.0, "worker", "release", json!([1, token(3)])),
            at(13.0, "worker", "claim", json!([1])),
            raw(14.0, json!(1)),
            at(15.0, "worker", "release", json!([1, token(3)])),
            at(16.0, "stranger", "release", json!([1, token(5)])),
            at(17.0, "worker", "release", json!(["1", token(5)])),
            raw(18.0, json!(1)),
        ],
    ));
}

/// The gate: a paused or expired run takes no task, claim or release, a
/// dead member may still read, and the expiry commits before the refusal.
#[test]
fn the_operation_gate_refuses_identically() {
    run_both(&scenario(
        staffed(5),
        [
            task(3.0, "first", "first", json!(null)),
            at(4.0, "worker", "claim", json!([1])),
            sql("UPDATE members SET status='dead' WHERE id='worker'"),
            raw(5.0, json!(1)),
            at(5.5, "worker", "task_raw", json!([1])),
            at(6.0, "worker", "release", json!([1, token(3)])),
            sql("UPDATE members SET status='live' WHERE id='worker'"),
            at(7.0, "worker", "task_create", json!(["late", "t", ["ok"]])),
            at(3_700.0, "worker", "claim", json!([1])),
            at(
                3_701.0,
                "worker",
                "task_create",
                json!(["later", "t", ["ok"]]),
            ),
            raw(3_702.0, json!(1)),
        ],
    ));
}

/// A completed dependency unblocks: the derived status is read against
/// each dependency's stored status.
#[test]
fn completed_dependencies_unblock_identically() {
    run_both(&scenario(
        staffed(5),
        [
            task(3.0, "first", "first", json!(null)),
            task(4.0, "second", "second", json!(null)),
            task(5.0, "third", "third", json!([1, 2])),
            sql("UPDATE tasks SET status='completed' WHERE id=1"),
            raw(6.0, json!(3)),
            at(7.0, "worker", "claim", json!([3])),
            sql("UPDATE tasks SET status='completed' WHERE id=2"),
            raw(8.0, json!(3)),
            task(9.0, "fourth", "fourth", json!([1, 2])),
            at(10.0, "worker", "claim", json!([3])),
        ],
    ));
}

/// The cap: a board holding 1000 tasks takes no new one, and a request
/// already answered still replays.
#[test]
fn task_board_full_at_1000_identically() {
    run_both(&scenario(
        staffed(5),
        [
            task(3.0, "early", "early", json!(null)),
            sql(
                "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i < 998) \
                 INSERT INTO tasks(title,acceptance,dependencies,status,evidence) \
                 SELECT 'bulk', '[\"ok\"]', '[]', 'ready', '[]' FROM n",
            ),
            task(4.0, "last", "last", json!(null)),
            task(5.0, "over", "over", json!(null)),
            task(6.0, "early", "early", json!(null)),
            raw(7.0, json!(1000)),
        ],
    ));
}

/// One call on each board, the same arguments at the same instant.
fn both(
    rust: &RustBoard,
    python: &mut PyBoard,
    member: &str,
    method: &str,
    args: Value,
    offset: f64,
) -> (Outcome, Outcome) {
    let args = args.to_string();
    let now = NOW + offset;
    (
        rust.call_text(member, method, &args, now),
        python.call(member, method, &args, now),
    )
}

/// Mixed writers: a `task_create` result the Rust board stored replays in
/// the Python board, and the reverse, on one file; each answers the stored
/// text (keys sorted) and conflicts on the other's payload.
#[test]
fn ledger_results_replay_across_implementations() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("swarm.sqlite");
    let rust = RustBoard::open(&database, dir.path());
    let mut python = PyBoard::start(&database, dir.path(), dir.path());
    for step in staffed(5) {
        let outcome = rust.call_text(&step.member, &step.method, &step.args, step.now);
        assert!(matches!(outcome, Outcome::Ok(_)), "{step:?}: {outcome:?}");
    }
    let created = rust.call_text(
        "worker",
        "task_create",
        r#"["rust", "by rust", ["é"], [] ]"#,
        NOW + 3.0,
    );
    let Outcome::Ok(created) = created else {
        panic!("{created:?}")
    };
    let (rust_replay, python_replay) = both(
        &rust,
        &mut python,
        "worker",
        "task_create",
        json!(["rust", "by rust", ["é"]]),
        4.0,
    );
    assert_eq!(rust_replay, python_replay);
    let Outcome::Ok(replayed) = python_replay else {
        panic!("{python_replay:?}")
    };
    assert_eq!(replayed, created, "the same task");
    assert_eq!(replayed["id"], json!(1));

    let from_python = python.call(
        "worker",
        "task_create",
        &json!(["python", "by python", ["ok"], [1]]).to_string(),
        NOW + 5.0,
    );
    assert!(matches!(from_python, Outcome::Ok(_)), "{from_python:?}");
    let (rust_replay, python_replay) = both(
        &rust,
        &mut python,
        "worker",
        "task_create",
        json!(["python", "by python", ["ok"], [1]]),
        6.0,
    );
    assert_eq!(rust_replay, python_replay);
    let Outcome::Ok(replayed) = rust_replay else {
        panic!("{rust_replay:?}")
    };
    assert_eq!(replayed["id"], json!(2));
    assert_eq!(replayed["status"], json!("blocked"));
    let (rust_conflict, python_conflict) = both(
        &rust,
        &mut python,
        "worker",
        "task_create",
        json!(["python", "other", ["ok"], [1]]),
        7.0,
    );
    assert_eq!(rust_conflict, python_conflict);
    assert_eq!(
        rust_conflict,
        Outcome::Refused("request id reused with different payload".to_owned())
    );
}
