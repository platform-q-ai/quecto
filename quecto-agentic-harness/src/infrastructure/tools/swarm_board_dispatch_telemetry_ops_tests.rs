//! #2303: every op the dispatcher serves records what it acted on, and a
//! member-facing op the caller's role in the run.
use serde_json::{Value, json};

use super::super::{BOARD_OPS, Method, SwarmBoardHandles, call};
use super::{Recorded, create_args, logged, only};
use crate::domain::swarm::BoardRole;
use std::sync::Arc;

/// Every `Method` the dispatcher has. The `match` is exhaustive, so a new
/// method does not compile until it is listed here too, and then
/// [`BOARD_OPS`] must name it for the test below to pass.
fn every_method() -> Vec<Method> {
    let all = vec![
        Method::Status,
        Method::Snapshot,
        Method::Admit,
        Method::Activate,
        Method::RecordLaunch,
        Method::ReleaseUnlaunched,
        Method::Socket,
        Method::TaskCreate,
        Method::Dependencies,
        Method::Claim,
        Method::Release,
        Method::Block,
        Method::Unblock,
        Method::Submit,
        Method::VerifyTask,
        Method::CreateRun,
        Method::BootstrapRun,
        Method::BootstrapJoin,
        Method::TaskRaw,
    ];
    for method in &all {
        match method {
            Method::Status
            | Method::Snapshot
            | Method::Admit
            | Method::Activate
            | Method::RecordLaunch
            | Method::ReleaseUnlaunched
            | Method::Socket
            | Method::TaskCreate
            | Method::Dependencies
            | Method::Claim
            | Method::Release
            | Method::Block
            | Method::Unblock
            | Method::Submit
            | Method::VerifyTask
            | Method::CreateRun
            | Method::BootstrapRun
            | Method::BootstrapJoin
            | Method::TaskRaw => {}
        }
    }
    all
}

/// The bootstrapped run on `handles`' board created, so it runs: the task
/// ops need a running run.
fn running(handles: &SwarmBoardHandles) {
    call(handles, "parent", "create_run", create_args()).unwrap();
}

/// A task the parent creates on `handles`' board (a running run), as
/// `request`.
fn task(handles: &SwarmBoardHandles, request: &str) -> Value {
    call(
        handles,
        "parent",
        "task_create",
        json!([request, "t", ["tests pass"]]),
    )
    .unwrap()
}

/// Task 1 on `handles`' board (a running run), claimed by the parent; the
/// claim's token.
fn claimed(handles: &SwarmBoardHandles) -> Value {
    running(handles);
    task(handles, "r1");
    call(handles, "parent", "claim", json!([1])).unwrap()["token"].clone()
}

/// Evidence for task 1's one criterion at revision `R1`.
fn evidence() -> Value {
    json!([{"artifact": "report", "revision": "R1"}])
}

/// What an answered call of `method` acted on, as its record must say:
/// `(args, task_id, message_id, cursor_moved)`, after the setup calls it
/// needs on `handles` (a run the parent bootstrapped). The `match` is
/// exhaustive, so a new method does not compile until its record's fields
/// are stated here (#2303 review M2). A task id is the stored row's, not
/// the argument as given: `"2"` acts on task 2.
fn acted_on(
    method: Method,
    handles: &SwarmBoardHandles,
) -> (Value, Option<i64>, Option<i64>, Option<bool>) {
    let admitted = || call(handles, "parent", "_admit", json!(["worker", "r1"])).unwrap();
    match method {
        Method::Status | Method::Snapshot => (json!([]), None, None, None),
        Method::Admit => (json!(["worker", "r1"]), None, None, None),
        Method::Activate => {
            admitted();
            (json!(["worker", "r1", 7, "s", "/w.sock"]), None, None, None)
        }
        Method::RecordLaunch => {
            admitted();
            (json!(["worker", "r1", 7, "s"]), None, None, None)
        }
        Method::ReleaseUnlaunched => {
            admitted();
            (json!(["worker"]), None, None, None)
        }
        Method::Socket => (json!(["/p.sock"]), None, None, None),
        Method::TaskCreate => {
            running(handles);
            (json!(["r1", "t", ["tests pass"]]), Some(1), None, None)
        }
        Method::Dependencies => {
            running(handles);
            task(handles, "r1");
            task(handles, "r2");
            (json!(["2", [1]]), Some(2), None, None)
        }
        Method::Claim => {
            running(handles);
            task(handles, "r1");
            (json!([1]), Some(1), None, None)
        }
        Method::Release => {
            running(handles);
            task(handles, "r1");
            let claimed = call(handles, "parent", "claim", json!([1])).unwrap();
            (json!([true, claimed["token"]]), Some(1), None, None)
        }
        Method::Block => {
            let token = claimed(handles);
            (json!(["1", token, "waiting"]), Some(1), None, None)
        }
        Method::Unblock => {
            let token = claimed(handles);
            call(handles, "parent", "block", json!([1, token, "waiting"])).unwrap();
            (json!([true, token, "resolved"]), Some(1), None, None)
        }
        Method::Submit => {
            let token = claimed(handles);
            (json!(["1", token, evidence()]), Some(1), None, None)
        }
        Method::VerifyTask => {
            let token = claimed(handles);
            call(handles, "parent", "submit", json!([1, token, evidence()])).unwrap();
            (json!([true, token, "R1"]), Some(1), None, None)
        }
        Method::CreateRun => (create_args(), None, None, None),
        Method::BootstrapRun => (json!([1, "s", null]), None, None, None),
        Method::BootstrapJoin => (json!([1, "s", null]), None, None, None),
        Method::TaskRaw => {
            running(handles);
            task(handles, "r1");
            (json!([1]), Some(1), None, None)
        }
    }
}

/// Every method, answered, records the task, the message and the cursor
/// move its own arm reports, per op.
#[test]
fn every_answered_op_records_what_it_acted_on() {
    for method in every_method() {
        let log = Arc::new(Recorded::default());
        let (_dir, handles) = logged(&log);
        call(&handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
        let (args, task_id, message_id, cursor_moved) = acted_on(method, &handles);
        log.clear();
        call(&handles, "parent", method.name(), args).unwrap();
        let recorded = only(&log);
        assert_eq!(recorded.op, method.name());
        assert_eq!(
            (recorded.task_id, recorded.message_id, recorded.cursor_moved),
            (task_id, message_id, cursor_moved),
            "{recorded:?}"
        );
    }
}

/// The harness's own ops record `host`; a member-facing op records the
/// caller's role in the run as its measure read it (#2303 reconcile): the
/// coordinator's, a worker's, and `null` for one refused before it read
/// any run.
#[test]
fn a_member_facing_op_records_the_callers_run_role() {
    let log = Arc::new(Recorded::default());
    let (_dir, handles) = logged(&log);
    call(&handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
    running(&handles);
    call(&handles, "parent", "_admit", json!(["worker", "r1"])).unwrap();
    call(
        &handles,
        "parent",
        "_activate",
        json!(["worker", "r1", 7, "s", "/w.sock"]),
    )
    .unwrap();
    assert_eq!(
        log.ops().last().map(|op| op.role),
        Some(Some(BoardRole::Host))
    );
    log.clear();
    task(&handles, "r1");
    assert_eq!(only(&log).role, Some(BoardRole::Coordinator));
    log.clear();
    call(&handles, "worker", "claim", json!([1])).unwrap();
    assert_eq!(only(&log).role, Some(BoardRole::Worker));
    log.clear();
    call(&handles, "worker", "claim", json!([1, 2])).unwrap_err();
    assert_eq!(only(&log).role, None, "refused while binding: no run read");
}

#[test]
fn board_ops_names_every_method_once() {
    let names: Vec<&str> = every_method().into_iter().map(Method::name).collect();
    let mut listed = BOARD_OPS.to_vec();
    listed.sort_unstable();
    let mut expected = names.clone();
    expected.sort_unstable();
    assert_eq!(listed, expected, "BOARD_OPS lists every method, once");
    for name in names {
        assert!(Method::parse(name).is_some(), "{name} parses");
    }
}
