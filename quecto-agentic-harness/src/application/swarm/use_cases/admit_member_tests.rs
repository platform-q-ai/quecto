use serde_json::{Value, json};

use super::AdmitMember;
use crate::application::swarm::board_test_support::{
    MemoryBoard, SteppingClock, member_row, running_board, stored_member,
};
use crate::application::swarm::dto::{AdmissionDecision, AdmitMemberRequest};
use crate::application::swarm::use_cases::OverRepository;
use crate::domain::swarm::{
    BoardError, MemberRecord, MemberState, RefusalKind, RunState, admission,
};

fn admit(actor: &str, member: Value, reservation: &str) -> AdmitMemberRequest {
    AdmitMemberRequest {
        actor: actor.to_owned(),
        member,
        reservation: json!(reservation),
    }
}

/// `swarm_policy_test.py::test_admission_retries_and_capacity_use_same_atomic_port`:
/// a retry under the same reservation answers the same row and writes
/// nothing, capacity refuses a third member, and only the first admission
/// records an event.
#[test]
fn admission_retries_and_capacity_use_same_atomic_port() {
    let board = MemoryBoard::with(running_board(100.0));
    let service = AdmitMember::new(board.clone(), SteppingClock::fixed(50.0));
    let first = service
        .execute(admit("parent", json!("worker"), "token"))
        .unwrap();
    assert_eq!(first.decision, AdmissionDecision::Reserved);
    let again = service
        .execute(admit("parent", json!("worker"), "token"))
        .unwrap();
    assert_eq!(again.row, first.row);
    assert_eq!(again.decision, AdmissionDecision::Retry);
    let refused = service
        .execute(admit("parent", json!("third"), "token3"))
        .unwrap_err();
    assert!(refused.message().contains("reuse"), "{refused}");
    assert_eq!(
        refused,
        BoardError::new(
            RefusalKind::MemberLimit,
            "swarm limit 2, current usage 2; reuse the existing pool"
        )
    );
    let state = board.snapshot();
    assert_eq!(state.members.len(), 2);
    assert_eq!(state.events.len(), 1);
    assert_eq!(state.events[0].action, "reserved");
    assert_eq!(state.events[0].detail, json!({"member": "worker"}));
    // `admission(repo.run(), first, 'token', 2, 100)`: at the deadline the
    // same admission is refused, as Python's final assertion checks.
    let run = state.run.expect("the board's run").record;
    let prior = MemberRecord {
        id: first.row.text("id").unwrap().to_owned(),
        status: first.row.text("status").map(MemberState::new),
        reservation: first.row.text("reservation").map(str::to_owned),
    };
    let refused = admission(&run, Some(&prior), &json!("token"), 2, 100.0).unwrap_err();
    assert!(refused.message().contains("no new admission"), "{refused}");
}

/// The reserving actor is the member's launcher (#1961), the actor of the
/// `reserved` event, and the row answered is the whole `dict(row)`.
#[test]
fn the_actor_is_recorded_as_launcher_and_the_whole_row_is_answered() {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    if let Some(run) = state.run.as_mut() {
        run.record.member_limit = 5;
    }
    let board = MemoryBoard::with(state);
    let service = AdmitMember::new(board.clone(), SteppingClock::new(&[10.0, 20.0, 30.0]));
    let admitted = service
        .execute(admit("worker", json!("child"), "r-child"))
        .unwrap();
    assert_eq!(
        admitted.row,
        stored_member(
            "child",
            json!("r-child"),
            "reserved",
            [json!(null), json!(null), json!(null), json!("worker")]
        )
    );
    let event = &board.snapshot().events[0];
    assert_eq!(
        (event.actor.as_str(), event.time),
        ("worker", 30.0),
        "the event is stamped on the clock's third reading, after expiry and admission"
    );
    assert_eq!(
        board.journal(),
        ["reserve_member child by worker", "event reserved"]
    );
    assert_eq!(
        board.transactions(),
        [false, false],
        "two transactions, neither creating"
    );
}

/// `bounded(member, 'member', 128)` runs before any transaction.
#[test]
fn a_member_name_is_bounded_before_the_board_is_opened() {
    let board = MemoryBoard::with(running_board(100.0));
    let service = AdmitMember::new(board.clone(), SteppingClock::fixed(50.0));
    for member in [
        json!("x".repeat(129)),
        json!(""),
        json!("   "),
        json!(5),
        json!(null),
        json!("é".repeat(65)),
    ] {
        assert_eq!(
            service
                .execute(admit("parent", member.clone(), "r"))
                .unwrap_err(),
            BoardError::new(
                RefusalKind::Invalid,
                "member must be nonempty and at most 128 bytes"
            ),
            "{member}"
        );
    }
    assert!(board.transactions().is_empty());
    let exact = service
        .execute(admit("parent", json!("x".repeat(128)), "r"))
        .unwrap();
    assert_eq!(exact.decision, AdmissionDecision::Reserved);
}

/// A run that is no longer admitting refuses with its status; a dead
/// actor is refused by the gate; a known identity under a new reservation
/// is refused once capacity allows it.
#[test]
fn admission_refusals_keep_pythons_text() {
    let mut paused = running_board(100.0);
    if let Some(run) = paused.run.as_mut() {
        run.record.status = Some(RunState::PAUSED);
    }
    let board = MemoryBoard::with(paused);
    let service = AdmitMember::new(board, SteppingClock::fixed(50.0));
    assert_eq!(
        service
            .execute(admit("parent", json!("w"), "r"))
            .unwrap_err(),
        BoardError::new(RefusalKind::NotRunning, "run is paused; no new admission")
    );

    let mut state = running_board(100.0);
    state.members.push(member_row("gone", "dead"));
    if let Some(run) = state.run.as_mut() {
        run.record.member_limit = 5;
    }
    let board = MemoryBoard::with(state);
    let service = AdmitMember::new(board, SteppingClock::fixed(50.0));
    assert_eq!(
        service.execute(admit("gone", json!("w"), "r")).unwrap_err(),
        BoardError::new(
            RefusalKind::NotMember,
            "invoking member is unknown or death confirmed"
        )
    );
    assert_eq!(
        service
            .execute(admit("parent", json!("gone"), "gone-reservation"))
            .unwrap_err(),
        BoardError::new(
            RefusalKind::IdentityTaken,
            "member identity already used; choose a stable new identity"
        ),
        "a dead member's own reservation is no retry"
    );
}

/// An expired run is ended by the gate's first transaction, which commits,
/// and admission then refuses the paused run.
#[test]
fn an_expired_run_commits_its_end_then_refuses_admission() {
    let board = MemoryBoard::with(running_board(100.0));
    let service = AdmitMember::new(board.clone(), SteppingClock::fixed(100.0));
    assert_eq!(
        service
            .execute(admit("parent", json!("w"), "r"))
            .unwrap_err(),
        BoardError::new(
            RefusalKind::BudgetExhausted,
            "run is paused; no new admission"
        )
    );
    let actions: Vec<String> = board
        .snapshot()
        .events
        .into_iter()
        .map(|event| event.action)
        .collect();
    assert_eq!(actions, ["stop", "paused"]);
}

/// Served over another repository (#2303 reconcile), admission writes to
/// that board alone, on the clock it was composed with.
#[test]
fn over_admits_on_the_given_board() {
    let composed_over = MemoryBoard::with(running_board(100.0));
    let other = MemoryBoard::with(running_board(100.0));
    let admitted = AdmitMember::new(composed_over.clone(), SteppingClock::fixed(50.0))
        .over(other.clone())
        .execute(admit("parent", json!("worker"), "token"))
        .unwrap();
    assert_eq!(admitted.decision, AdmissionDecision::Reserved);
    assert_eq!(other.snapshot().members.len(), 2);
    assert!(composed_over.transactions().is_empty());
    assert_eq!(composed_over.snapshot().members.len(), 1);
}
