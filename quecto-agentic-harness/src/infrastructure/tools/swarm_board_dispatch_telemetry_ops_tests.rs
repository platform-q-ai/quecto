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
        Method::Pause,
        Method::Resume,
        Method::ResumeExternal,
        Method::Close,
        Method::ExtendDeadline,
        Method::Stop,
        Method::ControlStatus,
        Method::UsageReport,
        Method::Complete,
        Method::RevalidateTask,
        Method::Amend,
        Method::Evidence,
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
            | Method::Pause
            | Method::Resume
            | Method::ResumeExternal
            | Method::Close
            | Method::ExtendDeadline
            | Method::Stop
            | Method::ControlStatus
            | Method::UsageReport
            | Method::Complete
            | Method::RevalidateTask
            | Method::Amend
            | Method::Evidence
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

/// The parent's accepted evidence for the run's one criterion (`t`, a
/// command check) at revision `R1`.
fn accepted(handles: &SwarmBoardHandles) {
    call(
        handles,
        "parent",
        "evidence",
        json!(["t", "report", "R1", "command", true]),
    )
    .unwrap();
}

/// What a call of `method` acted on, as its record must say:
/// `(args, task_id, message_id, cursor_moved)`, after the setup calls it
/// needs on `handles` (a run the parent bootstrapped). The `match` is
/// exhaustive, so a new method does not compile until its record's fields
/// are stated here (#2303 review M2). A task id is the stored row's, not
/// the argument as given: `"2"` acts on task 2. Every call answers but a
/// member's own `resume`, which the board always refuses (#2273).
fn acted_on(
    method: Method,
    handles: &SwarmBoardHandles,
) -> (Value, Option<i64>, Option<i64>, Option<bool>) {
    let admitted = || call(handles, "parent", "_admit", json!(["worker", "r1"])).unwrap();
    let stopped = || {
        running(handles);
        call(handles, "parent", "stop", json!(["blocked", "why"])).unwrap();
    };
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
        Method::Pause | Method::Stop | Method::ExtendDeadline | Method::ControlStatus => {
            running(handles);
            let args = match method {
                Method::Pause => json!(["hold"]),
                Method::Stop => json!(["blocked", "why"]),
                Method::ExtendDeadline => json!([60]),
                _ => json!([]),
            };
            (args, None, None, None)
        }
        Method::Resume | Method::UsageReport => (json!([]), None, None, None),
        Method::ResumeExternal | Method::Close => {
            stopped();
            (json!([]), None, None, None)
        }
        Method::Complete => {
            running(handles);
            accepted(handles);
            (json!(["R1"]), None, None, None)
        }
        Method::RevalidateTask => {
            let token = claimed(handles);
            call(handles, "parent", "submit", json!([1, token, evidence()])).unwrap();
            call(handles, "parent", "verify_task", json!([1, token, "R1"])).unwrap();
            let evidence = json!([{"artifact": "rerun", "revision": "R2"}]);
            (json!(["1", "R2", evidence]), Some(1), None, None)
        }
        Method::Amend => {
            running(handles);
            let criteria = json!([{"id": "t", "kind": "review", "description": "d"}]);
            (json!(["g", [], criteria, "why"]), None, None, None)
        }
        Method::Evidence => {
            running(handles);
            (
                json!(["t", "report", "R1", "command", true]),
                None,
                None,
                None,
            )
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
        let answer = call(&handles, "parent", method.name(), args);
        assert_eq!(
            answer.is_ok(),
            method != Method::Resume,
            "{}: {answer:?}",
            method.name()
        );
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

/// Run control (#2273): the coordinator's `pause`, `stop` and
/// `usage_report` record its role in the run; the supervisor's
/// `_control_status`, `_extend_deadline`, `_resume_external` and `_close`
/// record `host`; a member's own `resume`, refused before it reads any
/// run, records `null`.
#[test]
fn run_control_records_the_coordinator_or_the_host() {
    let log = Arc::new(Recorded::default());
    let (_dir, handles) = logged(&log);
    call(&handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
    running(&handles);
    for (method, args, role) in [
        ("usage_report", json!([]), Some(BoardRole::Coordinator)),
        ("_control_status", json!([]), Some(BoardRole::Host)),
        ("_extend_deadline", json!([60]), Some(BoardRole::Host)),
        ("pause", json!(["hold"]), Some(BoardRole::Coordinator)),
        ("_resume_external", json!([]), Some(BoardRole::Host)),
        (
            "stop",
            json!(["blocked", "why"]),
            Some(BoardRole::Coordinator),
        ),
        ("_close", json!([]), Some(BoardRole::Host)),
    ] {
        log.clear();
        call(&handles, "parent", method, args).unwrap();
        assert_eq!(only(&log).role, role, "{method}");
    }
    log.clear();
    call(&handles, "parent", "resume", json!([])).unwrap_err();
    assert_eq!(only(&log).role, None, "refused before any run read");
}

/// Completion (#2273): `complete`, `revalidate_task`, `amend` and
/// `evidence` are member-facing, so each records the caller's role in the
/// run: the coordinator's, or a worker's for its proposed evidence.
#[test]
fn completion_records_the_callers_run_role() {
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
    let token = claimed_by_worker(&handles);
    call(&handles, "worker", "submit", json!([1, token, evidence()])).unwrap();
    call(&handles, "parent", "verify_task", json!([1, token, "R1"])).unwrap();
    let criteria = json!([{"id": "t", "kind": "command", "description": "d"}]);
    for (member, method, args, role) in [
        (
            "parent",
            "amend",
            json!(["g", [], criteria, "why"]),
            BoardRole::Coordinator,
        ),
        (
            "worker",
            "evidence",
            json!(["t", "report", "R1", "command", true]),
            BoardRole::Worker,
        ),
        (
            "parent",
            "evidence",
            json!(["t", "report", "R1", "command", true]),
            BoardRole::Coordinator,
        ),
        (
            "parent",
            "revalidate_task",
            json!([1, "R1", evidence()]),
            BoardRole::Coordinator,
        ),
        ("parent", "complete", json!(["R1"]), BoardRole::Coordinator),
    ] {
        log.clear();
        call(&handles, member, method, args).unwrap();
        assert_eq!(only(&log).role, Some(role), "{member} {method}");
    }
}

/// Task 1 on `handles`' board (a running run), created by the parent and
/// claimed by the worker; the claim's token.
fn claimed_by_worker(handles: &SwarmBoardHandles) -> Value {
    task(handles, "r1");
    call(handles, "worker", "claim", json!([1])).unwrap()["token"].clone()
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
