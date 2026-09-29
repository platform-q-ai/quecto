//! #2303: every refusal the domain raises carries its stable kind, and a
//! `swarm_op` record serializes as the event log reads it.
use serde_json::{Value, json};

use super::*;
use crate::domain::audit::AuditEvent;
use crate::domain::swarm::records::{MemberState, RunState, TaskState};
use crate::domain::swarm::{
    Access, BoardError, MemberRecord, RunRecord, TaskRecord, admission, authorize, bounded,
    completion, criteria, require_budget, require_unsubmitted, revalidation, validate_extension,
};

fn run(status: RunState) -> RunRecord {
    RunRecord {
        status: Some(status),
        coordinator: Some("parent".into()),
        deadline: 100.0,
        member_limit: 1,
        outcome: None,
        outcome_reason: None,
    }
}

fn member(id: &str) -> MemberRecord {
    MemberRecord {
        id: id.into(),
        status: Some(MemberState::LIVE),
        reservation: Some("r".into()),
    }
}

fn task(status: TaskState) -> TaskRecord {
    TaskRecord {
        id: 1,
        status,
        owner: None,
        evidence: json!([]),
    }
}

fn kind<T: std::fmt::Debug>(result: Result<T, BoardError>) -> RefusalKind {
    result.expect_err("the domain refuses").kind()
}

const ACTIVE: Access = Access {
    active: true,
    coordinator: false,
    read_only: false,
};
const COORDINATOR: Access = Access {
    active: false,
    coordinator: true,
    read_only: false,
};

#[test]
fn every_domain_refusal_carries_its_kind() {
    let running = run(RunState::RUNNING);
    let parent = member("parent");
    let cases: Vec<(RefusalKind, RefusalKind)> = vec![
        (
            kind(authorize(None, "parent", Some(&parent), ACTIVE)),
            RefusalKind::RunMissing,
        ),
        (
            kind(authorize(
                Some(&running),
                "worker",
                Some(&member("worker")),
                COORDINATOR,
            )),
            RefusalKind::NotCoordinator,
        ),
        (
            kind(authorize(Some(&running), "ghost", None, ACTIVE)),
            RefusalKind::NotMember,
        ),
        (
            kind(authorize(
                Some(&run(RunState::PAUSED)),
                "parent",
                Some(&parent),
                ACTIVE,
            )),
            RefusalKind::NotRunning,
        ),
        (
            kind(require_budget(&running, 200.0)),
            RefusalKind::BudgetExhausted,
        ),
        (kind(validate_extension(&json!(0))), RefusalKind::Invalid),
        (
            kind(admission(&run(RunState::PAUSED), None, Some("r"), 0, 0.0)),
            RefusalKind::NotRunning,
        ),
        (
            kind(admission(&running, None, Some("r"), 1, 0.0)),
            RefusalKind::MemberLimit,
        ),
        (
            kind(admission(
                &RunRecord {
                    member_limit: 2,
                    ..running.clone()
                },
                Some(&MemberRecord {
                    reservation: Some("old".into()),
                    ..member("worker")
                }),
                Some("new"),
                1,
                0.0,
            )),
            RefusalKind::IdentityTaken,
        ),
        (
            kind(completion(&[], &[], &[], false, &json!(""))),
            RefusalKind::Invalid,
        ),
        (
            kind(completion(&[], &[], &[], false, &json!("rev"))),
            RefusalKind::CompletionUnmet,
        ),
        (
            kind(revalidation(
                &task(TaskState::READY),
                &json!("r"),
                &json!([]),
            )),
            RefusalKind::WrongState,
        ),
        (
            kind(revalidation(
                &task(TaskState::COMPLETED),
                &json!("r"),
                &json!([]),
            )),
            RefusalKind::Invalid,
        ),
        (
            kind(require_unsubmitted(&task(TaskState::SUBMITTED))),
            RefusalKind::Immutable,
        ),
        (
            kind(require_unsubmitted(&task(TaskState::new("lost")))),
            RefusalKind::WrongState,
        ),
        (kind(bounded(&json!(""), "title", 10)), RefusalKind::Invalid),
        (kind(criteria(&json!([]), 2)), RefusalKind::Invalid),
        (
            kind(criteria(&json!([{"kind": "vibes"}]), 20)),
            RefusalKind::Invalid,
        ),
    ];
    for (index, (found, expected)) in cases.into_iter().enumerate() {
        assert_eq!(found, expected, "case {index}");
    }
}

fn observation(outcome: BoardOpOutcome) -> BoardOpObservation {
    BoardOpObservation {
        op: "_snapshot".into(),
        actor_ref: "worker-1".into(),
        role: BoardRole::Host,
        run_id: Some("0123456789abcdef0123456789abcdef".into()),
        task_id: None,
        message_id: None,
        outcome,
        duration_us: 1_500,
        lock_wait_us: 400,
        busy: true,
        cursor_moved: false,
        result_bytes: 42,
    }
}

/// The record is flat: the outcome and its kind sit beside the other
/// fields, and it reads back as it was written.
#[test]
fn a_swarm_op_record_is_flat_and_round_trips() {
    let refused = AuditEvent::SwarmOp(observation(BoardOpOutcome::Refused {
        kind: RefusalKind::NotMember,
    }));
    let line = serde_json::to_value(&refused).unwrap();
    assert_eq!(
        line,
        json!({
            "event": "swarm_op",
            "op": "_snapshot",
            "actor_ref": "worker-1",
            "role": "host",
            "run_id": "0123456789abcdef0123456789abcdef",
            "outcome": "refused",
            "kind": "not_member",
            "duration_us": 1_500,
            "lock_wait_us": 400,
            "busy": true,
            "cursor_moved": false,
            "result_bytes": 42,
        })
    );
    let read: AuditEvent = serde_json::from_value(line).unwrap();
    assert_eq!(read, refused);
    let ok = AuditEvent::SwarmOp(observation(BoardOpOutcome::Ok));
    let line = serde_json::to_value(&ok).unwrap();
    assert_eq!(line["outcome"], "ok");
    assert_eq!(line.get("kind"), None);
    assert_eq!(serde_json::from_value::<AuditEvent>(line).unwrap(), ok);
}

/// Every kind serializes as the text `as_str` gives, which the tracing
/// records carry.
#[test]
fn every_kind_serializes_as_its_stable_text() {
    let kinds = [
        RefusalKind::RunMissing,
        RefusalKind::NotCoordinator,
        RefusalKind::NotMember,
        RefusalKind::NotRunning,
        RefusalKind::BudgetExhausted,
        RefusalKind::MemberLimit,
        RefusalKind::IdentityTaken,
        RefusalKind::RunExists,
        RefusalKind::CompletionUnmet,
        RefusalKind::StaleRevision,
        RefusalKind::Immutable,
        RefusalKind::WrongState,
        RefusalKind::NotOwner,
        RefusalKind::StaleToken,
        RefusalKind::ReservedByOther,
        RefusalKind::DependencyCycle,
        RefusalKind::Invalid,
        RefusalKind::Calling,
        RefusalKind::Contended,
        RefusalKind::StoreMissing,
        RefusalKind::Store,
        RefusalKind::Internal,
    ];
    for kind in kinds {
        assert_eq!(
            serde_json::to_value(kind).unwrap(),
            Value::String(kind.as_str().to_owned())
        );
    }
}
