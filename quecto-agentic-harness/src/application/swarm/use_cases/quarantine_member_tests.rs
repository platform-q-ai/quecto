use serde_json::{Value, json};

use super::QuarantineMember;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, SteppingClock, member_row, recorded, running_board, stored_member,
};
use crate::application::swarm::dto::{Quarantine, QuarantineMemberRequest};
use crate::application::swarm::use_cases::OverRepository;
use crate::domain::swarm::{BoardError, RefusalKind, RunState};

/// A live member launched by `launcher`.
fn launched(id: &str, launcher: &str) -> crate::application::swarm::dto::MemberRow {
    stored_member(
        id,
        json!(format!("{id}-r")),
        "live",
        [json!(7), json!("s"), Value::Null, json!(launcher)],
    )
}

/// A running run with `worker` launched by `parent`, and `other` too.
fn board() -> BoardState {
    let mut state = running_board(1_000.0);
    state.members.push(launched("worker", "parent"));
    state.members.push(launched("other", "parent"));
    state
}

fn quarantine(actor: &str, member: &str) -> QuarantineMemberRequest {
    QuarantineMemberRequest {
        actor: actor.to_owned(),
        member: json!(member),
    }
}

fn actions(board: &MemoryBoard) -> Vec<String> {
    let events = board.snapshot().events;
    events.into_iter().map(|event| event.action).collect()
}

/// Only the launcher records, and only a grace after its first
/// observation: another member's observation writes nothing.
#[test]
fn only_the_launcher_records_and_only_after_the_grace() {
    let board = MemoryBoard::with(board());
    let at = |now: f64| QuarantineMember::new(board.clone(), SteppingClock::fixed(now));
    assert_eq!(
        at(10.0).execute(quarantine("other", "worker")).unwrap(),
        Quarantine::NotLauncher
    );
    assert!(actions(&board).is_empty());
    assert_eq!(
        at(10.0).execute(quarantine("parent", "worker")).unwrap(),
        Quarantine::GracePending
    );
    assert_eq!(
        at(19.9).execute(quarantine("parent", "worker")).unwrap(),
        Quarantine::GracePending
    );
    assert_eq!(actions(&board), ["scope_observed"], "one per observer");
    assert_eq!(
        at(20.0).execute(quarantine("parent", "worker")).unwrap(),
        Quarantine::Recorded
    );
    assert_eq!(
        actions(&board),
        ["scope_observed", "scope_unknown", "stop", "paused"]
    );
    let events = board.snapshot().events;
    assert_eq!(
        events[1].detail,
        json!({
            "member": "worker",
            "reason": "harness exited; execution scope unconfirmed; discard environment"
        })
    );
    let run = board.snapshot().run.unwrap().record;
    assert_eq!(
        (run.status, run.outcome.as_deref()),
        (Some(RunState::PAUSED), Some("failed"))
    );
    // Recorded once: a later observation leaves the member as it is.
    assert_eq!(
        at(30.0).execute(quarantine("parent", "worker")).unwrap(),
        Quarantine::AlreadyLost
    );
    assert_eq!(actions(&board).len(), 4);
}

/// Once the launcher is itself dead, any member may record the loss
/// after the grace; a member without a launcher is recorded at once.
#[test]
fn a_lost_launcher_or_none_lets_any_member_record() {
    let mut state = board();
    state.members.push(launched("nested", "worker"));
    state.members[1] = member_row("worker", "dead");
    let board = MemoryBoard::with(state);
    let at = |now: f64| QuarantineMember::new(board.clone(), SteppingClock::fixed(now));
    assert_eq!(
        at(10.0).execute(quarantine("other", "nested")).unwrap(),
        Quarantine::GracePending
    );
    assert_eq!(
        at(20.0).execute(quarantine("other", "nested")).unwrap(),
        Quarantine::Recorded
    );
    let launcherless = MemoryBoard::with(board_with_worker());
    assert_eq!(
        QuarantineMember::new(launcherless.clone(), SteppingClock::fixed(5.0))
            .execute(quarantine("worker", "parent"))
            .unwrap(),
        Quarantine::Recorded
    );
    assert_eq!(actions(&launcherless), ["scope_unknown", "stop", "paused"]);
}

fn board_with_worker() -> BoardState {
    let mut state = running_board(1_000.0);
    state.members.push(launched("worker", "parent"));
    state
}

/// A dead or unknown member, or one recorded lost after its latest
/// activation, is already lost: nothing is written.
#[test]
fn an_already_lost_member_writes_nothing() {
    let mut state = board();
    state.members.push(member_row("gone", "dead"));
    state.events.push(recorded(
        "parent",
        "scope_unknown",
        json!({"member": "other"}),
    ));
    let board = MemoryBoard::with(state);
    let service = QuarantineMember::new(board.clone(), SteppingClock::fixed(5.0));
    for member in ["gone", "stranger", "other"] {
        assert_eq!(
            service.execute(quarantine("parent", member)).unwrap(),
            Quarantine::AlreadyLost,
            "{member}"
        );
    }
    assert_eq!(actions(&board), ["scope_unknown"]);
}

/// `unknown_member_status_is_not_alive`: a member whose status is unknown
/// or NULL is already lost, where Python observes it.
#[test]
fn an_unknown_member_status_is_already_lost() {
    let mut state = board();
    state.members.push(member_row("zombie", "zombie"));
    let mut null = launched("null", "parent");
    null.columns[2].1 = Value::Null;
    state.members.push(null);
    let board = MemoryBoard::with(state);
    let service = QuarantineMember::new(board.clone(), SteppingClock::fixed(5.0));
    for member in ["zombie", "null"] {
        assert_eq!(
            service.execute(quarantine("parent", member)).unwrap(),
            Quarantine::AlreadyLost
        );
    }
    assert!(actions(&board).is_empty());
}

/// The gate refuses a caller whose death is confirmed; served over
/// another repository, the op writes to that board alone.
#[test]
fn the_gate_refuses_a_dead_caller_and_over_serves_the_given_board() {
    let mut state = board();
    state.members[1] = member_row("worker", "dead");
    let board = MemoryBoard::with(state);
    let service = QuarantineMember::new(board.clone(), SteppingClock::fixed(5.0));
    assert_eq!(
        service.execute(quarantine("worker", "parent")).unwrap_err(),
        BoardError::new(
            RefusalKind::NotMember,
            "invoking member is unknown or death confirmed"
        )
    );
    let other = MemoryBoard::with(board_with_worker());
    service
        .over(other.clone())
        .execute(quarantine("worker", "parent"))
        .unwrap();
    assert_eq!(actions(&other).len(), 3);
    assert!(actions(&board).is_empty());
}
