//! Differential scenarios (#2277, epic #2265): the summaries `create`,
//! `_join` and `_bootstrap` answer with, the read models' argument checks,
//! the paused-run table, an unreadable store, and the dispatcher's method
//! set against Python's `Workbench`.
//!
//! Python's `create` commits and then calls `summary()`, which can still
//! refuse; so can the join's closing `coordinator.summary()` in every
//! branch, the already-live one included (#2307, #2310 review
//! obligations).
use std::collections::BTreeSet;

use serde_json::{Value, json};

use crate::swarm_board_diff_loss::{answer, refusal};
use crate::swarm_board_diff_membership::{HOUR, at};
use crate::swarm_board_diff_messages::joined;
use crate::swarm_board_diff_reads::{full, summary, task};
use crate::swarm_board_diff_runs::NOW;
use crate::swarm_board_diff_runs::swarm_board_diff::python::workbench_methods;
use crate::swarm_board_diff_runs::swarm_board_diff::scenario::{
    Hold, Step, corrupt, held, run_both, sql, step,
};
use quecto::infrastructure::tools::swarm_board_dispatch::BOARD_OPS;

const UNKNOWN: &str = "invoking member is unknown or death confirmed";

fn contract(deadline: f64) -> Value {
    json!({
        "goal": "read models",
        "constraints": ["c"],
        "criteria": [{"id": "t", "kind": "command", "description": "test"}],
        "member_limit": 5,
        "deadline": deadline,
    })
}

fn bootstrap(offset: f64, member: &str, pid: i64) -> Step {
    at(offset, member, "_bootstrap", json!([pid, "start", null]))
}

/// `create` answers the creator's summary, over the placeholder or fresh;
/// when the creator's row is gone it commits the run and still refuses
/// ("committed, then refused", #2307).
#[test]
fn create_answers_the_summary_and_commits_before_its_refusal() {
    let fresh = [
        step("parent", "create", contract(NOW + HOUR), NOW),
        full(1.0, "parent"),
    ];
    run_both(&fresh);
    assert_eq!(answer(&fresh, 0), answer(&fresh, 1));
    let over = [
        step("parent", "bootstrap_run", json!([1, "s", null]), NOW),
        at(1.0, "parent", "create", contract(NOW + HOUR)),
    ];
    run_both(&over);
    assert_eq!(answer(&over, 1)["status"], json!("running"));
    let orphaned = [
        step("parent", "bootstrap_run", json!([1, "s", null]), NOW),
        sql("DELETE FROM members WHERE id='parent'"),
        at(1.0, "parent", "create", contract(NOW + HOUR)),
        at(2.0, "supervisor", "_status", json!([])),
    ];
    run_both(&orphaned);
    assert_eq!(refusal(&orphaned, 2), UNKNOWN);
    assert_eq!(answer(&orphaned, 3)["status"], json!("running"));
}

/// `bootstrap_returns_the_coordinator_summary_identically`: every branch
/// of `_bootstrap` (the placeholder, a new member admitted, the same live
/// process, a reactivation) answers the coordinator's summary.
#[test]
fn bootstrap_returns_the_coordinator_summary_identically() {
    let steps = [
        bootstrap(0.0, "parent", 1),
        bootstrap(1.0, "worker", 2),
        bootstrap(2.0, "worker", 2),
        at(3.0, "parent", "create", contract(NOW + HOUR)),
        bootstrap(4.0, "late", 3),
        at(
            5.0,
            "worker",
            "_bootstrap",
            json!([9, "restart", null, "wrong"]),
        ),
        full(6.0, "parent"),
    ];
    run_both(&steps);
    let placeholder = answer(&steps, 0);
    assert_eq!(
        (&placeholder["status"], &placeholder["coordinator"]),
        (&json!("setup"), &json!("parent"))
    );
    assert_eq!(answer(&steps, 2), answer(&steps, 1));
    assert_eq!(
        refusal(&steps, 5),
        "launch reservation does not match invoking process"
    );
}

/// The join's closing summary runs as the coordinator in every branch:
/// the already-live branch writes nothing and still refuses a coordinator
/// that is nobody (NULL, or without a row), and still ends an expired run
/// first (#2310).
#[test]
fn the_join_answers_the_coordinators_summary_in_every_branch() {
    let live = joined([at(3.0, "worker", "_join", json!(["res-w", 11, "t", null]))]);
    run_both(&live);
    assert_eq!(answer(&live, 3)["coordinator"], json!("parent"));
    for edit in [
        "UPDATE run SET coordinator=NULL",
        "DELETE FROM members WHERE id='parent'",
    ] {
        let steps = joined([
            sql(edit),
            at(3.0, "worker", "_join", json!(["res-w", 11, "t", null])),
            at(4.0, "worker", "_bootstrap", json!([11, "t", null])),
        ]);
        run_both(&steps);
        assert_eq!(refusal(&steps, 4), UNKNOWN, "{edit}");
        assert_eq!(refusal(&steps, 5), UNKNOWN, "{edit}");
    }
    let expired = joined([
        at(
            HOUR + 1.0,
            "worker",
            "_join",
            json!(["res-w", 11, "t", null]),
        ),
        at(HOUR + 2.0, "supervisor", "_status", json!([])),
    ]);
    run_both(&expired);
    assert_eq!(
        (
            &answer(&expired, 3)["status"],
            &answer(&expired, 3)["outcome"]
        ),
        (&json!("paused"), &json!("budget-exhausted"))
    );
    let admitted = joined([
        at(3.0, "fresh", "_join", json!([null, 13, "f", "/f.sock"])),
        at(
            4.0,
            "worker",
            "_join",
            json!(["other-reservation", 99, "x", null]),
        ),
    ]);
    run_both(&admitted);
    assert_eq!(
        refusal(&admitted, 4),
        "launch reservation does not match invoking process"
    );
}

/// The read models' arguments, checked before the gate as Python checks
/// them (`type(x) is int`, no boolean, float or text), and the gate: a
/// stranger is refused, a dead member may read.
#[test]
fn read_model_arguments_and_gates_are_checked_as_python_does() {
    let mut steps = joined([task(3.0, "worker", "task")]);
    for args in [
        json!({"after": 0, "limit": 0}),
        json!({"after": 0, "limit": 101}),
        json!({"after": 0, "limit": true}),
        json!({"after": 0.0}),
        json!({"after": 3, "limit": 100}),
        json!([1, 1]),
    ] {
        steps.push(at(4.0, "parent", "events", args));
    }
    for args in [
        json!({"offset": -1}),
        json!({"offset": false}),
        json!({"limit": 0}),
        json!({"limit": 1.0}),
        json!({"offset": 0, "limit": 100}),
        json!([1, 1]),
    ] {
        steps.push(at(5.0, "parent", "tasks", args));
    }
    for task_id in [json!(1), json!("1"), json!(true), json!(99), json!(null)] {
        steps.push(at(6.0, "parent", "task", json!([task_id])));
    }
    steps.extend([
        summary(7.0, "stranger", Value::Null),
        at(7.1, "stranger", "events", json!([])),
        at(7.2, "stranger", "tasks", json!([])),
        at(7.3, "stranger", "task", json!([1])),
        sql("UPDATE members SET status='dead' WHERE id='worker'"),
        full(8.0, "worker"),
        at(8.1, "worker", "events", json!([])),
        at(8.2, "worker", "tasks", json!([])),
        at(8.3, "worker", "task", json!([1])),
        summary(9.0, "worker", json!(4)),
        summary(9.1, "worker", json!(0)),
    ]);
    run_both(&steps);
    assert_eq!(refusal(&steps, 19), "unknown task");
    assert_eq!(refusal(&steps, 21), UNKNOWN);
}

/// Reads on a paused run answer as Python does; `create`, `_join` and
/// `_bootstrap` on it are refused as Python refuses them (the paused-run
/// table).
#[test]
fn every_read_model_op_on_a_paused_run() {
    for paused in [
        vec![at(3.0, "parent", "pause", json!(["hold"]))],
        vec![sql("UPDATE run SET status='paused'")],
    ] {
        let mut steps = joined(paused);
        steps.extend([
            full(4.0, "parent"),
            summary(4.1, "parent", json!(4)),
            at(4.2, "parent", "events", json!([])),
            at(4.3, "parent", "tasks", json!([])),
            at(4.4, "parent", "create", contract(NOW + HOUR)),
            at(4.5, "late", "_join", json!([null, 5, "l", null])),
            at(4.6, "late", "_bootstrap", json!([5, "l", null])),
            at(4.7, "worker", "_join", json!(["res-w", 11, "t", null])),
        ]);
        run_both(&steps);
    }
}

/// `test_contention_and_corruption_fail_explicitly`: a write lock held
/// past the timeout refuses a task, and a store that is no database
/// refuses the summary, each with the same text on both boards.
#[test]
fn contention_and_corruption_fail_explicitly() {
    let steps = joined([
        held(Hold::Throughout, task(3.0, "worker", "task")),
        corrupt(),
        full(4.0, "parent"),
    ]);
    run_both(&steps);
    assert!(refusal(&steps, 3).starts_with("coordination store unavailable or contended: "));
    assert_eq!(
        refusal(&steps, 5),
        "coordination store unavailable or contended: file is not a database"
    );
}

/// The Workbench helpers that are no board method: each takes the open
/// store connection (`db`) or is a pure check an op runs inside its own
/// transaction, so none is callable from outside the board.
const INTERNAL_HELPERS: [&str; 15] = [
    "_criteria",
    "_dependencies",
    "_end_by_loss",
    "_grace_elapsed",
    "_liveness_watch",
    "_lost",
    "_message_id",
    "_notify_revoked",
    "_owned",
    "_owner_views",
    "_reopen",
    "_retire",
    "_task",
    "_with_owner_liveness",
    "owned_tasks",
];

/// The dispatcher test-only names, which no `Workbench` method has.
const TEST_ONLY: [&str; 4] = ["create_run", "bootstrap_run", "bootstrap_join", "task_raw"];

/// Every method of Python's `Workbench` (`dir(Workbench)`, filtered to the
/// callables `swarm.py` and `swarm_tasks.py` define) is a dispatcher
/// method or a named internal helper, and the dispatcher serves nothing
/// else.
#[test]
fn the_dispatcher_serves_every_workbench_method() {
    let python: BTreeSet<String> = workbench_methods().into_iter().collect();
    let served: BTreeSet<String> = BOARD_OPS
        .iter()
        .filter(|op| !TEST_ONLY.contains(op))
        .chain(INTERNAL_HELPERS.iter())
        .map(|name| (*name).to_owned())
        .collect();
    assert_eq!(python, served);
}

/// `test_the_advertised_contact_is_a_send_the_board_accepts`: the contact
/// a claimed task advertises is the `send` the board accepts, delivered to
/// the owner.
#[test]
fn the_advertised_contact_is_a_send_the_board_accepts() {
    let steps = joined([
        task(3.0, "worker", "task"),
        at(4.0, "worker", "claim", json!([1])),
        at(5.0, "parent", "task", json!([1])),
        at(
            6.0,
            "parent",
            "send",
            json!(["dependency-question-1", "worker", "which schema?"]),
        ),
        at(7.0, "worker", "inbox", json!([])),
    ]);
    run_both(&steps);
    assert_eq!(
        answer(&steps, 5)["contact"],
        json!("board.send(request, 'worker', body)")
    );
    assert_eq!(answer(&steps, 6)["status"], json!("accepted"));
    let inbox = answer(&steps, 7);
    assert_eq!(
        (&inbox[0]["sender"], &inbox[0]["body"]),
        (&json!("parent"), &json!("which schema?"))
    );
}

/// `test_message_columns_are_migrated_into_an_older_store`: a store from
/// before #1837 gains the message columns on its next open, on both
/// boards alike.
#[test]
fn message_columns_are_migrated_into_an_older_store() {
    let steps = [
        sql(
            "CREATE TABLE run (id TEXT PRIMARY KEY, goal TEXT, constraints TEXT, criteria TEXT,
              coordinator TEXT, integrator TEXT, member_limit INTEGER, deadline REAL, status TEXT);
             CREATE TABLE members (id TEXT PRIMARY KEY, reservation TEXT UNIQUE, status TEXT, pid INTEGER, started TEXT, socket TEXT);
             CREATE TABLE tasks (id INTEGER PRIMARY KEY, title TEXT, acceptance TEXT, dependencies TEXT, status TEXT, owner TEXT, token TEXT, evidence TEXT, blocker TEXT);
             CREATE TABLE files (path TEXT PRIMARY KEY, task INTEGER, owner TEXT, claim TEXT, token TEXT);
             CREATE TABLE messages (id INTEGER PRIMARY KEY, sender TEXT, recipient TEXT, body TEXT, status TEXT);
             CREATE TABLE evidence (criterion TEXT, artifact TEXT, revision TEXT, kind TEXT, actor TEXT, accepted INTEGER, PRIMARY KEY(criterion, actor));
             CREATE TABLE requests (actor TEXT, request TEXT, payload TEXT, result TEXT, PRIMARY KEY(actor, request));
             CREATE TABLE events (id INTEGER PRIMARY KEY, actor TEXT, time REAL, action TEXT, detail TEXT);",
        ),
        step("parent", "create", contract(NOW + HOUR), NOW),
        at(1.0, "parent", "send", json!(["m", "parent", "to self", "abc1"])),
        at(2.0, "parent", "inbox", json!([])),
        at(3.0, "parent", "withdraw", json!([1])),
        at(4.0, "parent", "inbox", json!([true])),
    ];
    run_both(&steps);
    assert_eq!(answer(&steps, 3)[0]["revision"], json!("abc1"));
    assert_eq!(answer(&steps, 5)[0]["status"], json!("withdrawn"));
}

/// `test_the_launcher_column_is_migrated_into_an_older_store`: a store
/// whose members have no launcher column gains it, and an admission
/// records its launcher, on both boards alike.
#[test]
fn the_launcher_column_is_migrated_into_an_older_store() {
    let steps = joined([
        sql("ALTER TABLE members DROP COLUMN launcher"),
        full(3.0, "parent"),
        at(4.0, "parent", "_admit", json!(["late", "reservation-late"])),
        full(5.0, "parent"),
    ]);
    run_both(&steps);
    let members = answer(&steps, 6)["members"].clone();
    let launcher = |id: &str| {
        members
            .as_array()
            .unwrap()
            .iter()
            .find(|member| member["id"] == json!(id))
            .map(|member| member["launcher"].clone())
    };
    assert_eq!(launcher("late"), Some(json!("parent")));
    assert_eq!(launcher("parent"), Some(Value::Null));
}
