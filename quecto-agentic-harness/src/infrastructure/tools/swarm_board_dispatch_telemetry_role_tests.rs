//! #2313 review M2: a live member the operation gate authorised records
//! its role in the run even when the op then refused it: a stale claim
//! token, a task in the wrong state, or a refusal after the op's writes
//! committed. A refusal of the gate itself (no member, not the
//! coordinator, no run) records no role.
use std::sync::Arc;

use serde_json::{Value, json};

use super::{Recorded, SwarmBoardHandles, call, create_args, logged, only};
use crate::domain::swarm::{BoardOpObservation, BoardOpOutcome, BoardRole, RefusalKind};

/// A running run with the worker admitted and activated by the parent,
/// holding task 1.
fn running(handles: &SwarmBoardHandles) {
    call(handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
    call(handles, "parent", "create_run", create_args()).unwrap();
    call(handles, "parent", "_admit", json!(["worker", "r1"])).unwrap();
    call(
        handles,
        "parent",
        "_activate",
        json!(["worker", "r1", 7, "s", "/w.sock"]),
    )
    .unwrap();
    call(
        handles,
        "parent",
        "task_create",
        json!(["t1", "t", ["tests pass"]]),
    )
    .unwrap();
}

/// The one record of `member`'s refused call of `method`.
fn refused(
    log: &Recorded,
    handles: &SwarmBoardHandles,
    member: &str,
    method: &str,
    args: Value,
) -> BoardOpObservation {
    log.clear();
    call(handles, member, method, args).unwrap_err();
    only(log)
}

fn kind(record: &BoardOpObservation) -> RefusalKind {
    match record.outcome {
        BoardOpOutcome::Refused { kind, .. } => kind,
        BoardOpOutcome::Ok => panic!("an answer: {record:?}"),
    }
}

/// A live worker's submit with a stale token, and its claim of a task
/// already claimed: refused after the gate authorised it, so its role is
/// recorded.
#[test]
fn a_live_workers_refusal_after_the_gate_records_worker() {
    let log = Arc::new(Recorded::default());
    let (_dir, handles) = logged(&log);
    running(&handles);
    call(&handles, "worker", "claim", json!([1])).unwrap();
    let evidence = json!([{"artifact": "report", "revision": "R1"}]);
    let stale = refused(
        &log,
        &handles,
        "worker",
        "submit",
        json!([1, "stale", evidence]),
    );
    assert_eq!(kind(&stale), RefusalKind::StaleToken, "{stale:?}");
    assert_eq!(stale.role, Some(BoardRole::Worker), "{stale:?}");
    let claimed = refused(&log, &handles, "worker", "claim", json!([1]));
    assert_eq!(kind(&claimed), RefusalKind::WrongState, "{claimed:?}");
    assert_eq!(claimed.role, Some(BoardRole::Worker), "{claimed:?}");
    let parent = refused(&log, &handles, "parent", "claim", json!([1]));
    assert_eq!(kind(&parent), RefusalKind::WrongState, "{parent:?}");
    assert_eq!(parent.role, Some(BoardRole::Coordinator), "{parent:?}");
}

/// A refusal of the gate itself records no role: a worker that is not
/// the coordinator asking for the coordinator's op, and a stranger.
#[test]
fn a_refusal_of_the_gate_itself_records_no_role() {
    let log = Arc::new(Recorded::default());
    let (_dir, handles) = logged(&log);
    running(&handles);
    let evidence = json!([{"artifact": "report", "revision": "R1"}]);
    let worker = refused(
        &log,
        &handles,
        "worker",
        "verify_task",
        json!([1, "t", "R1"]),
    );
    assert_eq!(kind(&worker), RefusalKind::NotCoordinator, "{worker:?}");
    assert_eq!(worker.role, None, "{worker:?}");
    let stranger = refused(
        &log,
        &handles,
        "stranger",
        "submit",
        json!([1, "t", evidence]),
    );
    assert_eq!(kind(&stranger), RefusalKind::NotMember, "{stranger:?}");
    assert_eq!(stranger.role, None, "{stranger:?}");
}

/// `create`'s closing summary refused after the run's writes committed
/// (a task row edited from outside): the gate had authorised the
/// coordinator, so the committed refusal records its role.
#[test]
fn a_committed_refusal_after_the_gate_records_the_role() {
    let log = Arc::new(Recorded::default());
    let (dir, handles) = logged(&log);
    call(&handles, "parent", "bootstrap_run", json!([1, "s", null])).unwrap();
    rusqlite::Connection::open(dir.path().join("swarm.sqlite"))
        .unwrap()
        .execute(
            "INSERT INTO tasks(id,title,status) VALUES(1,'t','edited')",
            [],
        )
        .unwrap();
    let committed = refused(&log, &handles, "parent", "create", create_args());
    assert!(
        matches!(
            committed.outcome,
            BoardOpOutcome::Refused {
                committed: true,
                ..
            }
        ),
        "{committed:?}"
    );
    assert_eq!(
        committed.role,
        Some(BoardRole::Coordinator),
        "{committed:?}"
    );
}

/// #2313 final review: a member whose death was confirmed still passes
/// the gate for a read (read-only access admits it), so its reads record
/// the role it holds in the run, `worker`; its mutations are refused by
/// the gate itself and record no role.
#[test]
fn a_dead_members_reads_record_its_role_and_its_mutations_none() {
    let log = Arc::new(Recorded::default());
    let (dir, handles) = logged(&log);
    running(&handles);
    rusqlite::Connection::open(dir.path().join("swarm.sqlite"))
        .unwrap()
        .execute("UPDATE members SET status='dead' WHERE id='worker'", [])
        .unwrap();
    log.clear();
    call(&handles, "worker", "summary", json!([null])).unwrap();
    let read = only(&log);
    assert_eq!(read.role, Some(BoardRole::Worker), "{read:?}");
    let task = json!(["t2", "t", ["tests pass"]]);
    let refused = refused(&log, &handles, "worker", "task_create", task);
    assert_eq!(kind(&refused), RefusalKind::NotMember, "{refused:?}");
    assert_eq!(refused.role, None, "{refused:?}");
}
