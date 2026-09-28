//! Ported from `tests/swarm_policy_test.py`; messages asserted by string.
use serde_json::{Value, json};

use super::*;
use crate::domain::swarm_board::records::{
    CriterionKind, EvidenceRef, MemberState, RunState, TaskState,
};

fn run(status: RunState) -> RunRecord {
    RunRecord {
        status,
        coordinator: "parent".into(),
        deadline: 100.0,
        member_limit: 2,
        outcome: None,
        outcome_reason: None,
    }
}

fn running() -> RunRecord {
    run(RunState::RUNNING)
}

fn member(id: &str, status: MemberState, reservation: &str) -> MemberRecord {
    MemberRecord {
        id: id.into(),
        status,
        reservation: Some(reservation.into()),
    }
}

fn task(status: TaskState, revisions: &[&str]) -> TaskRecord {
    TaskRecord {
        id: 1,
        status,
        owner: None,
        evidence: revisions
            .iter()
            .map(|revision| EvidenceRef {
                artifact: "log".into(),
                revision: (*revision).into(),
            })
            .collect(),
    }
}

fn message<T: std::fmt::Debug>(result: Result<T, BoardError>) -> String {
    result.expect_err("the policy refuses").to_string()
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
const MUTATE: Access = Access {
    active: false,
    coordinator: false,
    read_only: false,
};
const READ: Access = Access {
    active: false,
    coordinator: false,
    read_only: true,
};

#[test]
fn authorize_refuses_a_missing_run() {
    let live = member("parent", MemberState::LIVE, "p");
    assert_eq!(
        message(authorize(None, "parent", Some(&live), READ)),
        "coordination run missing"
    );
}

#[test]
fn authorize_refuses_a_non_coordinator_for_coordinator_ops() {
    let live = member("worker", MemberState::LIVE, "w");
    assert_eq!(
        message(authorize(
            Some(&running()),
            "worker",
            Some(&live),
            COORDINATOR
        )),
        "only the designated coordinator may do this"
    );
    let parent = member("parent", MemberState::LIVE, "p");
    assert_eq!(
        authorize(Some(&running()), "parent", Some(&parent), COORDINATOR),
        Ok(())
    );
}

#[test]
fn authorize_lets_a_dead_member_read_but_not_mutate() {
    let dead = member("parent", MemberState::DEAD, "p");
    assert_eq!(
        authorize(Some(&running()), "parent", Some(&dead), READ),
        Ok(())
    );
    for access in [MUTATE, ACTIVE, COORDINATOR] {
        assert_eq!(
            message(authorize(Some(&running()), "parent", Some(&dead), access)),
            "invoking member is unknown or death confirmed"
        );
    }
    for access in [MUTATE, READ] {
        assert_eq!(
            message(authorize(Some(&running()), "parent", None, access)),
            "invoking member is unknown or death confirmed"
        );
    }
    for status in [MemberState::LIVE, MemberState::RESERVED] {
        let alive = member("parent", status, "p");
        assert_eq!(
            authorize(Some(&running()), "parent", Some(&alive), ACTIVE),
            Ok(())
        );
    }
}

#[test]
fn authorize_checks_in_python_order() {
    // Missing run before coordinator, coordinator before member, member before activity.
    let dead = member("worker", MemberState::DEAD, "w");
    assert_eq!(
        message(authorize(None, "worker", None, COORDINATOR)),
        "coordination run missing"
    );
    assert_eq!(
        message(authorize(
            Some(&run(RunState::PAUSED)),
            "worker",
            Some(&dead),
            Access {
                active: true,
                coordinator: true,
                read_only: false
            }
        )),
        "only the designated coordinator may do this"
    );
    assert_eq!(
        message(authorize(
            Some(&run(RunState::PAUSED)),
            "parent",
            Some(&dead),
            ACTIVE
        )),
        "invoking member is unknown or death confirmed"
    );
}

#[test]
fn authorize_names_the_held_outcome_when_paused() {
    let live = member("parent", MemberState::LIVE, "p");
    let mut paused = run(RunState::PAUSED);
    paused.outcome = Some("failed".into());
    paused.outcome_reason = Some("harness lost".into());
    assert_eq!(
        message(authorize(Some(&paused), "parent", Some(&live), ACTIVE)),
        "run is paused (failed: harness lost); no new work permitted"
    );
    paused.outcome_reason = None;
    assert_eq!(
        message(authorize(Some(&paused), "parent", Some(&live), ACTIVE)),
        "run is paused (failed: no reason); no new work permitted"
    );
    paused.outcome_reason = Some(String::new());
    assert_eq!(describe(&paused), "paused (failed: no reason)");
    assert_eq!(
        authorize(Some(&paused), "parent", Some(&live), MUTATE),
        Ok(())
    );
}

#[test]
fn describe_is_the_bare_status_without_a_held_outcome() {
    assert_eq!(describe(&run(RunState::PAUSED)), "paused");
    let mut empty = run(RunState::PAUSED);
    empty.outcome = Some(String::new());
    assert_eq!(describe(&empty), "paused");
    let mut failed = run(RunState::FAILED);
    failed.outcome = Some("failed".into());
    assert_eq!(describe(&failed), "failed");
    assert_eq!(describe(&run(RunState::new("odd"))), "odd");
    let live = member("parent", MemberState::LIVE, "p");
    assert_eq!(
        message(authorize(
            Some(&run(RunState::SETUP)),
            "parent",
            Some(&live),
            ACTIVE
        )),
        "run is setup; no new work permitted"
    );
}

#[test]
fn expired_only_while_running() {
    let at = running();
    assert!(expired(&at, 100.0), "a deadline equal to now is expired");
    assert!(expired(&at, 101.0));
    assert!(!expired(&at, 99.5));
    assert!(
        !expired(&run(RunState::PAUSED), 500.0),
        "a paused run past its deadline is not expired"
    );
    assert!(!expired(&run(RunState::SETUP), 500.0));
    assert_eq!(
        message(require_budget(&at, 100.0)),
        "run is paused (budget-exhausted: deadline); no new work permitted"
    );
    assert_eq!(require_budget(&at, 99.0), Ok(()));
    assert_eq!(require_budget(&run(RunState::PAUSED), 500.0), Ok(()));
}

#[test]
fn admission_is_idempotent_for_the_same_live_reservation() {
    for status in [MemberState::LIVE, MemberState::RESERVED] {
        let prior = member("worker", status, "token");
        assert_eq!(
            admission(&running(), Some(&prior), "token", 1, 50.0),
            Ok(false)
        );
    }
    assert_eq!(
        admission(&run(RunState::SETUP), None, "token", 1, 500.0),
        Ok(true)
    );
}

#[test]
fn admission_counts_capacity_before_identity_reuse() {
    let reused = member("worker", MemberState::LIVE, "old");
    assert_eq!(
        message(admission(&running(), Some(&reused), "new", 2, 50.0)),
        "swarm limit 2, current usage 2; reuse the existing pool"
    );
    // The same live reservation is answered before capacity (Python order).
    let same = member("worker", MemberState::LIVE, "token");
    assert_eq!(
        admission(&running(), Some(&same), "token", 2, 50.0),
        Ok(false)
    );
    assert_eq!(
        message(admission(&running(), Some(&reused), "new", 1, 50.0)),
        "member identity already used; choose a stable new identity"
    );
    let dead = member("worker", MemberState::DEAD, "token");
    assert_eq!(
        message(admission(&running(), Some(&dead), "token", 1, 50.0)),
        "member identity already used; choose a stable new identity"
    );
    assert_eq!(
        message(admission(&running(), None, "token", 3, 50.0)),
        "swarm limit 2, current usage 3; reuse the existing pool"
    );
    assert_eq!(admission(&running(), None, "token", 1, 50.0), Ok(true));
}

#[test]
fn admission_refuses_stopped_and_expired_runs() {
    for status in [
        RunState::PAUSED,
        RunState::SUCCEEDED,
        RunState::BLOCKED,
        RunState::FAILED,
        RunState::CANCELLED,
        RunState::BUDGET_EXHAUSTED,
    ] {
        let expected = format!("run is {}; no new admission", status.as_str());
        let same = member("worker", MemberState::LIVE, "token");
        assert_eq!(
            message(admission(&run(status), Some(&same), "token", 0, 0.0)),
            expected
        );
    }
    let same = member("worker", MemberState::LIVE, "token");
    assert_eq!(
        message(admission(&running(), Some(&same), "token", 1, 100.0)),
        "run is running; no new admission"
    );
}

fn criteria() -> Vec<Criterion> {
    vec![Criterion {
        id: "test".into(),
        kind: CriterionKind::Command,
        description: "tests pass".into(),
    }]
}

fn accepted(revision: &str) -> Vec<EvidenceRow> {
    vec![EvidenceRow {
        criterion: "test".into(),
        revision: revision.into(),
        kind: CriterionKind::Command,
        accepted: true,
    }]
}

#[test]
fn completion_rejects_each_unsatisfied_requirement() {
    let r2 = json!("R2");
    let done = vec![task(TaskState::COMPLETED, &["R2"])];
    const EVIDENCE: &str =
        "completion requires accepted evidence at the current revision for every criterion";
    const SETTLE: &str = "settle outstanding work and file reservations before success";
    const STALE: &str = "task evidence refers to stale revision";
    for revision in [json!(null), json!(""), json!("  \t"), json!(2)] {
        assert_eq!(
            message(completion(
                &criteria(),
                &accepted("R2"),
                &done,
                false,
                &revision
            )),
            "completion revision required"
        );
    }
    let rows: [(
        Vec<Criterion>,
        Vec<EvidenceRow>,
        bool,
        Vec<TaskRecord>,
        &str,
    ); 5] = [
        (vec![], accepted("R2"), false, done.clone(), EVIDENCE),
        (criteria(), vec![], false, done.clone(), EVIDENCE),
        (criteria(), accepted("R2"), true, done.clone(), SETTLE),
        (
            criteria(),
            accepted("R2"),
            false,
            vec![task(TaskState::SUBMITTED, &[])],
            SETTLE,
        ),
        (
            criteria(),
            accepted("R2"),
            false,
            vec![task(TaskState::COMPLETED, &[])],
            STALE,
        ),
    ];
    for (criteria, evidence, reserved, tasks, expected) in rows {
        assert_eq!(
            message(completion(&criteria, &evidence, &tasks, reserved, &r2)),
            expected
        );
    }
    assert_eq!(
        message(completion(
            &criteria(),
            &accepted("R2"),
            &[task(TaskState::COMPLETED, &["R2", "R1"])],
            false,
            &r2
        )),
        STALE
    );
    assert_eq!(
        completion(&criteria(), &accepted("R2"), &done, false, &r2),
        Ok(RunState::SUCCEEDED)
    );
    assert_eq!(
        completion(&criteria(), &accepted("R2"), &[], false, &r2),
        Ok(RunState::SUCCEEDED)
    );
}

#[test]
fn completion_requires_accepted_evidence_of_the_matching_kind() {
    const EVIDENCE: &str =
        "completion requires accepted evidence at the current revision for every criterion";
    let r2 = json!("R2");
    let mut rejected = accepted("R2");
    rejected[0].accepted = false;
    let mut review = accepted("R2");
    review[0].kind = CriterionKind::Review;
    let mut other = accepted("R2");
    other[0].criterion = "lint".into();
    for evidence in [rejected, review, other, accepted("R1")] {
        assert_eq!(
            message(completion(&criteria(), &evidence, &[], false, &r2)),
            EVIDENCE
        );
    }
    let mut two = criteria();
    two.push(Criterion {
        id: "lint".into(),
        kind: CriterionKind::Review,
        description: "reviewed".into(),
    });
    assert_eq!(
        message(completion(&two, &accepted("R2"), &[], false, &r2)),
        EVIDENCE
    );
    let mut both = accepted("R2");
    both.push(EvidenceRow {
        criterion: "lint".into(),
        revision: "R2".into(),
        kind: CriterionKind::Review,
        accepted: true,
    });
    assert_eq!(
        completion(&two, &both, &[], false, &r2),
        Ok(RunState::SUCCEEDED)
    );
}

#[test]
fn revalidation_requires_completed_task_and_matching_revision() {
    let r2 = json!("R2");
    let good = json!([{"artifact": "rerun", "revision": "R2", "extra": 1}]);
    for status in [
        TaskState::READY,
        TaskState::CLAIMED,
        TaskState::BLOCKED,
        TaskState::SUBMITTED,
    ] {
        assert_eq!(
            message(revalidation(&task(status, &["R1"]), &r2, &good)),
            "only completed tasks may be revalidated"
        );
    }
    let completed = task(TaskState::COMPLETED, &["R1"]);
    let required = "new artifact and revision evidence required";
    for (revision, evidence) in [
        (json!(null), good.clone()),
        (json!(" "), good.clone()),
        (json!(2), good.clone()),
        (r2.clone(), json!([])),
        (r2.clone(), json!({"artifact": "rerun", "revision": "R2"})),
        (r2.clone(), json!(null)),
    ] {
        assert_eq!(
            message(revalidation(&completed, &revision, &evidence)),
            required
        );
    }
    let must_match = "new artifact evidence must match the revalidated revision";
    for evidence in [
        json!(["rerun"]),
        json!([{"revision": "R2"}]),
        json!([{"artifact": 5, "revision": "R2"}]),
        json!([{"artifact": " ", "revision": "R2"}]),
        json!([{"artifact": "rerun", "revision": "R1"}]),
        json!([{"artifact": "rerun"}]),
        json!([{"artifact": "rerun", "revision": "R2"}, {"artifact": "log", "revision": "R1"}]),
    ] {
        assert_eq!(
            message(revalidation(&completed, &r2, &evidence)),
            must_match
        );
    }
    assert_eq!(revalidation(&completed, &r2, &good), Ok(&good));
}

#[test]
fn resume_blockers_list_lost_coordinator_then_deadline_then_budget() {
    assert_eq!(
        resume_blockers(100.0, 100.0, "pause", Some("parent")),
        vec![
            "relaunch the lost coordinator 'parent' into the retained environment before resuming"
                .to_string(),
            "extend the deadline (swarm_control extend) before resuming".to_string(),
            "raise or disable the token budget (swarm_control usage_budget) before resuming"
                .to_string(),
        ]
    );
    assert_eq!(
        resume_blockers(101.0, 100.0, "warn", None),
        Vec::<String>::new()
    );
    assert_eq!(
        resume_blockers(101.0, 100.0, "allow", Some("")),
        Vec::<String>::new()
    );
    assert_eq!(
        resume_blockers(99.0, 100.0, "allow", None),
        vec!["extend the deadline (swarm_control extend) before resuming".to_string()]
    );
}

#[test]
fn validate_extension_bounds() {
    let refused = "deadline extension must be 1..604800 seconds";
    for seconds in [
        json!(0),
        json!(604801),
        json!(1.0),
        json!(3600.5),
        json!(-1),
        json!(true),
        json!("60"),
        json!(null),
    ] {
        assert_eq!(message(validate_extension(&seconds)), refused, "{seconds}");
    }
    assert_eq!(validate_extension(&json!(1)), Ok(1));
    assert_eq!(validate_extension(&json!(604800)), Ok(604800));
    assert_eq!(message(validate_extension(&Value::from(u64::MAX))), refused);
}

#[test]
fn require_unsubmitted_message() {
    assert_eq!(
        message(require_unsubmitted(&task(TaskState::SUBMITTED, &["R1"]))),
        "submitted evidence is immutable; release and reclaim before revising"
    );
    for status in [
        TaskState::READY,
        TaskState::CLAIMED,
        TaskState::BLOCKED,
        TaskState::COMPLETED,
    ] {
        assert_eq!(require_unsubmitted(&task(status, &[])), Ok(()));
    }
}

#[test]
fn outcome_constants_match_python() {
    assert_eq!(
        PROPOSED_OUTCOMES,
        ["succeeded", "blocked", "failed", "budget-exhausted"]
    );
    assert_eq!(
        STOP_STATUSES,
        ["blocked", "failed", "budget-exhausted", "cancelled"]
    );
}

#[test]
fn unknown_statuses_found_in_a_file_are_refused_affirmatively() {
    // Guards allow known values only. The board never writes these statuses;
    // Python's denylist would have let an unknown member status mutate.
    let odd = member("parent", MemberState::new("zombie"), "p");
    assert_eq!(
        message(authorize(Some(&running()), "parent", Some(&odd), MUTATE)),
        "invoking member is unknown or death confirmed"
    );
    assert_eq!(
        authorize(Some(&running()), "parent", Some(&odd), READ),
        Ok(())
    );
    let odd_prior = member("worker", MemberState::new("zombie"), "token");
    assert_eq!(
        message(admission(&running(), Some(&odd_prior), "token", 1, 50.0)),
        "member identity already used; choose a stable new identity"
    );
    assert_eq!(
        message(require_unsubmitted(&task(TaskState::new("odd"), &[]))),
        "submitted evidence is immutable; release and reclaim before revising"
    );
    assert_eq!(
        message(admission(&run(RunState::new("odd")), None, "token", 0, 0.0)),
        "run is odd; no new admission"
    );
}
