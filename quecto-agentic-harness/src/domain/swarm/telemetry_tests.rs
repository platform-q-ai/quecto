//! #2303: every refusal the domain raises carries its stable kind, and a
//! `swarm_op` record serializes as the event log reads it.
use serde_json::{Value, json};

use super::*;
use crate::domain::audit::AuditEvent;
use crate::domain::swarm::records::{MemberState, RunState, TaskState};
use crate::domain::swarm::{
    Access, BoardError, MemberExit, MemberRecord, RefusalKind, RunRecord, TaskRecord, admission,
    authorize, bounded, completion, criteria, require_budget, require_unsubmitted, revalidation,
    run_already, run_already_held, validate_extension,
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
            kind(admission(&run(RunState::PAUSED), None, &json!("r"), 0, 0.0)),
            RefusalKind::NotRunning,
        ),
        (
            kind(admission(&running, None, &json!("r"), 1, 0.0)),
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
                &json!("new"),
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
            kind(require_unsubmitted("submitted")),
            RefusalKind::Immutable,
        ),
        (kind(require_unsubmitted("lost")), RefusalKind::WrongState),
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
        role: Some(BoardRole::Host),
        run_id: Some("0123456789abcdef0123456789abcdef".into()),
        task_id: None,
        message_id: None,
        outcome,
        duration_us: 1_500,
        lock_wait_us: Some(400),
        busy_wait_us: Some(250),
        busy: Some(true),
        cursor_moved: Some(false),
        result_bytes: 42,
        decision: None,
        detail: BoardOpDetail::NONE,
    }
}

/// The record is flat: the outcome and its kind sit beside the other
/// fields, and it reads back as it was written.
#[test]
fn a_swarm_op_record_is_flat_and_round_trips() {
    let refused = AuditEvent::SwarmOp(observation(BoardOpOutcome::Refused {
        kind: RefusalKind::NotMember,
        committed: false,
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
            "busy_wait_us": 250,
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

/// Nothing measured is written as `null`, never as a zero that reads as
/// a real measure; nor is a cursor an op has none of written as unmoved.
#[test]
fn an_unmeasured_op_writes_null_waits() {
    let line = serde_json::to_value(AuditEvent::SwarmOp(BoardOpObservation {
        lock_wait_us: None,
        busy_wait_us: None,
        busy: None,
        cursor_moved: None,
        ..observation(BoardOpOutcome::Ok)
    }))
    .unwrap();
    for field in ["lock_wait_us", "busy_wait_us", "busy", "cursor_moved"] {
        assert_eq!(line.get(field), Some(&Value::Null), "{field}: {line}");
    }
}

fn paused(outcome: &str) -> RunRecord {
    RunRecord {
        outcome: Some(outcome.into()),
        outcome_reason: Some("deadline".into()),
        ..run(RunState::PAUSED)
    }
}

/// A run paused (or ended) as `budget-exhausted` refuses new work and new
/// members as `budget_exhausted`, not `not_running` (#2303 review M4),
/// with Python's text unchanged; another pause stays `not_running`.
#[test]
fn a_budget_exhausted_run_refuses_as_budget_exhausted() {
    let parent = member("parent");
    let refused = authorize(
        Some(&paused("budget-exhausted")),
        "parent",
        Some(&parent),
        ACTIVE,
    )
    .unwrap_err();
    assert_eq!(refused.kind(), RefusalKind::BudgetExhausted);
    assert_eq!(
        refused.message(),
        "run is paused (budget-exhausted: deadline); no new work permitted"
    );
    let ended = authorize(
        Some(&run(RunState::BUDGET_EXHAUSTED)),
        "parent",
        Some(&parent),
        ACTIVE,
    )
    .unwrap_err();
    assert_eq!(ended.kind(), RefusalKind::BudgetExhausted);
    assert_eq!(
        ended.message(),
        "run is budget-exhausted; no new work permitted"
    );
    let blocked = authorize(Some(&paused("blocked")), "parent", Some(&parent), ACTIVE).unwrap_err();
    assert_eq!(blocked.kind(), RefusalKind::NotRunning);

    let refused = admission(&paused("budget-exhausted"), None, &json!("r"), 0, 0.0).unwrap_err();
    assert_eq!(refused.kind(), RefusalKind::BudgetExhausted);
    assert_eq!(refused.message(), "run is paused; no new admission");
    // Still running, but past its deadline.
    let expired = admission(&run(RunState::RUNNING), None, &json!("r"), 0, 200.0).unwrap_err();
    assert_eq!(expired.kind(), RefusalKind::BudgetExhausted);
    assert_eq!(expired.message(), "run is running; no new admission");
    let blocked = admission(&paused("blocked"), None, &json!("r"), 0, 0.0).unwrap_err();
    assert_eq!(blocked.kind(), RefusalKind::NotRunning);
}

/// `stop`'s `run already …` refusals (#2273) keep Python's text and
/// refuse as `budget_exhausted` when the run's budget is spent, paused or
/// ended as `budget-exhausted`, and as `not_running` otherwise, a NULL
/// status included.
#[test]
fn a_stop_over_an_ended_run_refuses_by_its_budget() {
    let null = RunRecord {
        status: None,
        ..run(RunState::RUNNING)
    };
    for (record, described, expected) in [
        (
            paused("budget-exhausted"),
            "paused (budget-exhausted: deadline)",
            RefusalKind::BudgetExhausted,
        ),
        (
            run(RunState::BUDGET_EXHAUSTED),
            "budget-exhausted",
            RefusalKind::BudgetExhausted,
        ),
        (
            paused("blocked"),
            "paused (blocked: deadline)",
            RefusalKind::NotRunning,
        ),
        (
            run(RunState::CANCELLED),
            "cancelled",
            RefusalKind::NotRunning,
        ),
        (null, "None", RefusalKind::NotRunning),
    ] {
        let refused = run_already(&record);
        assert_eq!(
            (refused.kind(), refused.message()),
            (expected, format!("run already {described}").as_str())
        );
        let held = run_already_held(&record);
        let text = format!(
            "run already {described}; only the supervisor can resume or close it, \
             and op=cancel_run cancels it"
        );
        assert_eq!((held.kind(), held.message()), (expected, text.as_str()));
    }
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
        RefusalKind::NotFound,
        RefusalKind::CapacityFull,
        RefusalKind::SupervisorOnly,
        RefusalKind::LaunchConflict,
        RefusalKind::RequestIdReused,
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

/// #2303 round-4 review L3: a run id is recorded only when it is the
/// `uuid4().hex` the boards generate: 32 lowercase hex digits.
#[test]
fn only_a_generated_run_id_is_a_board_run_id() {
    assert!(board_run_id("0123456789abcdef0123456789abcdef"));
    for edited in [
        "",
        "0123456789abcdef0123456789abcde",
        "0123456789abcdef0123456789abcdef0",
        "0123456789ABCDEF0123456789ABCDEF",
        "0123456789abcdef0123456789abcdeg",
        "sk-livedeadbeef0001abcdefghijklm",
        "0123456789abcdef 123456789abcdef",
        "é123456789abcdef0123456789abcde",
    ] {
        assert!(!board_run_id(edited), "{edited:?}");
    }
}

/// A member-facing op's role is the caller's in the run (#2303 reconcile):
/// the coordinator, then the integrator, and any other member a worker.
#[test]
fn a_callers_run_role_is_read_from_the_run() {
    use super::run_role;
    assert_eq!(
        run_role("parent", Some("parent"), Some("parent"), true),
        Some(BoardRole::Coordinator)
    );
    assert_eq!(
        run_role("merger", Some("parent"), Some("merger"), true),
        Some(BoardRole::Integrator)
    );
    assert_eq!(
        run_role("worker", Some("parent"), Some("merger"), true),
        Some(BoardRole::Worker)
    );
    assert_eq!(
        run_role("worker", None, None, true),
        Some(BoardRole::Worker)
    );
}

/// #2313: a caller the board did not accept as a member of the run (a
/// stranger, or a member whose death was confirmed) holds no role, not
/// `worker`, whoever the run names.
#[test]
fn a_caller_that_is_no_member_holds_no_run_role() {
    use super::run_role;
    for (member, coordinator, integrator) in [
        ("stranger", Some("parent"), Some("merger")),
        ("parent", Some("parent"), Some("parent")),
        ("merger", Some("parent"), Some("merger")),
        ("worker", None, None),
    ] {
        assert_eq!(
            run_role(member, coordinator, integrator, false),
            None,
            "{member}"
        );
    }
}

/// A served op's decision and detail (#2277 review M1) sit flat beside
/// the other fields, each left out when it does not apply, and read back
/// as written; a record without them (an older writer's) reads as none.
#[test]
fn a_decision_and_its_detail_are_recorded_as_kinds_and_counts() {
    let recorded = AuditEvent::SwarmOp(BoardOpObservation {
        decision: Some("coordinator_confirmed".into()),
        detail: BoardOpDetail {
            run_status: Some(RunStatusKind::BudgetExhausted),
            exit: Some(MemberExit::Abrupt),
            reservations_retained: Some(2),
            ended_by_loss: Some(true),
            owners_scanned: Some(3),
            page_size: Some(4),
            has_more: Some(false),
            fast_path_defeated: Some(true),
            placeholder_created: Some(false),
        },
        ..observation(BoardOpOutcome::Ok)
    });
    let line = serde_json::to_value(&recorded).unwrap();
    assert_eq!(line["decision"], "coordinator_confirmed");
    assert_eq!(
        (
            &line["run_status"],
            &line["exit"],
            &line["reservations_retained"],
            &line["ended_by_loss"]
        ),
        (
            &json!("budget_exhausted"),
            &json!("abrupt"),
            &json!(2),
            &json!(true)
        )
    );
    // #2277 review M2: the read models' counts and flags.
    assert_eq!(
        (
            &line["owners_scanned"],
            &line["page_size"],
            &line["has_more"],
            &line["fast_path_defeated"],
            &line["placeholder_created"]
        ),
        (
            &json!(3),
            &json!(4),
            &json!(false),
            &json!(true),
            &json!(false)
        )
    );
    assert_eq!(
        serde_json::from_value::<AuditEvent>(line).unwrap(),
        recorded
    );
    let bare = serde_json::to_value(AuditEvent::SwarmOp(observation(BoardOpOutcome::Ok))).unwrap();
    for field in [
        "decision",
        "run_status",
        "exit",
        "reservations_retained",
        "ended_by_loss",
        "owners_scanned",
        "page_size",
        "has_more",
        "fast_path_defeated",
        "placeholder_created",
        "committed",
    ] {
        assert_eq!(bare.get(field), None, "{field}: {bare}");
    }
    let AuditEvent::SwarmOp(read) = serde_json::from_value(bare).unwrap() else {
        panic!("a swarm_op record");
    };
    assert_eq!((read.decision, read.detail), (None, BoardOpDetail::NONE));
}

/// Only the statuses the board writes are recorded as themselves; a NULL
/// or edited status is `unknown`, so its text never is.
#[test]
fn a_run_status_is_recorded_as_a_kind_of_the_boards_own() {
    let cases = [
        (Some(RunState::SETUP), RunStatusKind::Setup),
        (Some(RunState::RUNNING), RunStatusKind::Running),
        (Some(RunState::PAUSED), RunStatusKind::Paused),
        (Some(RunState::SUCCEEDED), RunStatusKind::Succeeded),
        (Some(RunState::BLOCKED), RunStatusKind::Blocked),
        (Some(RunState::FAILED), RunStatusKind::Failed),
        (Some(RunState::CANCELLED), RunStatusKind::Cancelled),
        (
            Some(RunState::BUDGET_EXHAUSTED),
            RunStatusKind::BudgetExhausted,
        ),
        (Some(RunState::new("sk-ant-edited")), RunStatusKind::Unknown),
        (Some(RunState::new("Running")), RunStatusKind::Unknown),
        (None, RunStatusKind::Unknown),
    ];
    for (status, kind) in cases {
        assert_eq!(RunStatusKind::of(status.as_ref()), kind, "{status:?}");
    }
}

/// A decision kind is lowercase snake_case: no text passes as one.
#[test]
fn a_decision_kind_is_snake_case_only() {
    for kind in ["recorded", "grace_pending", "a1"] {
        assert!(decision_kind(kind), "{kind}");
    }
    for text in ["", "Recorded", "grace pending", "sk-ant", "é"] {
        assert!(!decision_kind(text), "{text}");
    }
}

/// A refusal after the op's writes committed (#2277 review M2: `create`'s
/// summary) is marked, and reads back as written; a plain refusal carries
/// no marker.
#[test]
fn a_committed_refusal_is_marked() {
    let committed = AuditEvent::SwarmOp(observation(BoardOpOutcome::Refused {
        kind: RefusalKind::NotMember,
        committed: true,
    }));
    let line = serde_json::to_value(&committed).unwrap();
    assert_eq!(
        (&line["outcome"], &line["kind"], &line["committed"]),
        (&json!("refused"), &json!("not_member"), &json!(true))
    );
    assert_eq!(
        serde_json::from_value::<AuditEvent>(line).unwrap(),
        committed
    );
}
