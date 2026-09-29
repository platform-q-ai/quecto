use serde_json::{Value, json};

use super::ConfirmMemberDead;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, SteppingClock, StoredFile, member_row, running_board, stored_task,
};
use crate::application::swarm::dto::{ConfirmMemberDeadRequest, DeathConfirmation};
use crate::application::swarm::use_cases::OverRepository;
use crate::domain::swarm::{BoardError, RefusalKind, RunState};

/// `worker` owns tasks 1 (claimed), 2 (completed) and a reservation of
/// task 1; `other` owns task 3.
fn board() -> BoardState {
    let mut state = running_board(1_000.0);
    state.members.push(member_row("worker", "live"));
    state.members.push(member_row("other", "live"));
    state.tasks = vec![
        stored_task(1, "claimed", json!([]), Some("worker")),
        stored_task(2, "completed", json!([]), Some("worker")),
        stored_task(3, "claimed", json!([]), Some("other")),
    ];
    for (path, task, owner) in [("a", 1, "worker"), ("b", 3, "other")] {
        state.files.push(StoredFile {
            path: path.to_owned(),
            task,
            owner: owner.to_owned(),
            claim: "c".to_owned(),
            token: "t".to_owned(),
        });
    }
    state
}

fn dead(actor: &str, member: &str, exit: Value) -> ConfirmMemberDeadRequest {
    ConfirmMemberDeadRequest {
        actor: actor.to_owned(),
        member: json!(member),
        exit,
    }
}

fn statuses(state: &BoardState) -> Vec<(Option<String>, Option<String>)> {
    state
        .tasks
        .iter()
        .map(|task| {
            (
                task.text("status").map(str::to_owned),
                task.text("blocker").map(str::to_owned),
            )
        })
        .collect()
}

/// An orderly exit: the member is dead, its active work blocks, its
/// reservations go, and the run keeps going.
#[test]
fn an_orderly_exit_blocks_the_work_and_releases_the_reservations() {
    let board = MemoryBoard::with(board());
    let service = ConfirmMemberDead::new(board.clone(), SteppingClock::fixed(5.0));
    assert_eq!(
        service
            .execute(dead("parent", "worker", json!("orderly")))
            .unwrap(),
        DeathConfirmation::Confirmed { coordinator: false }
    );
    let state = board.snapshot();
    assert_eq!(state.members[1].text("status"), Some("dead"));
    let blocker = "worker death confirmed; coordinator recovery required";
    assert_eq!(
        statuses(&state),
        [
            (Some("blocked".to_owned()), Some(blocker.to_owned())),
            (Some("completed".to_owned()), None),
            (Some("claimed".to_owned()), None),
        ]
    );
    assert_eq!(state.files.len(), 1, "only the other member's is left");
    assert_eq!(
        state.events[0].detail,
        json!({"member": "worker", "exit": "orderly", "reservations_retained": 0})
    );
    assert_eq!(state.run.unwrap().record.status, Some(RunState::RUNNING));
    assert_eq!(
        service
            .execute(dead("parent", "worker", json!("abrupt")))
            .unwrap(),
        DeathConfirmation::AlreadyDead
    );
    assert_eq!(board.snapshot().events.len(), 1);
}

/// An abrupt exit retains the reservations and says so.
#[test]
fn an_abrupt_exit_retains_the_reservations_and_says_so() {
    let board = MemoryBoard::with(board());
    ConfirmMemberDead::new(board.clone(), SteppingClock::fixed(5.0))
        .execute(dead("parent", "worker", json!("abrupt")))
        .unwrap();
    let state = board.snapshot();
    assert_eq!(state.files.len(), 2);
    assert_eq!(
        state.tasks[0].text("blocker"),
        Some(
            "worker death confirmed (abrupt exit; reservations retained); coordinator recovery required"
        )
    );
    assert_eq!(
        state.events[0].detail,
        json!({
            "member": "worker",
            "exit": "abrupt",
            "reservations_retained": 1,
            "reason": "abrupt harness exit; orphaned tool processes may still write reserved paths"
        })
    );
}

/// The exit kind is checked before the gate: nothing is opened for a
/// kind that is not the text `orderly` or `abrupt`, whoever calls.
#[test]
fn the_exit_kind_is_checked_before_the_gate() {
    let board = MemoryBoard::with(board());
    let service = ConfirmMemberDead::new(board.clone(), SteppingClock::fixed(5.0));
    let refused = BoardError::new(RefusalKind::Invalid, "exit kind must be orderly or abrupt");
    for exit in [json!("Orderly"), Value::Null, json!(["abrupt"]), json!(1)] {
        assert_eq!(
            service
                .execute(dead("stranger", "worker", exit))
                .unwrap_err(),
            refused
        );
    }
    assert!(board.transactions().is_empty());
}

/// The coordinator's confirmed death ends the run by loss.
#[test]
fn the_coordinators_death_ends_the_run() {
    let board = MemoryBoard::with(board());
    assert_eq!(
        ConfirmMemberDead::new(board.clone(), SteppingClock::fixed(5.0))
            .execute(dead("worker", "parent", json!("orderly")))
            .unwrap(),
        DeathConfirmation::Confirmed { coordinator: true }
    );
    let state = board.snapshot();
    let run = state.run.unwrap().record;
    assert_eq!(
        (
            run.status,
            run.outcome.as_deref(),
            run.outcome_reason.as_deref()
        ),
        (
            Some(RunState::PAUSED),
            Some("failed"),
            Some("coordinator death confirmed")
        )
    );
    let actions: Vec<_> = state
        .events
        .iter()
        .map(|event| event.action.as_str())
        .collect();
    assert_eq!(actions, ["death_confirmed", "stop", "paused"]);
}

/// `unknown_member_status_is_not_alive`: a member whose status is unknown
/// or NULL is already dead, where Python confirms its death.
#[test]
fn an_unknown_member_status_is_already_dead() {
    let mut state = board();
    state.members.push(member_row("zombie", "zombie"));
    let mut null = member_row("null", "live");
    null.columns[2].1 = Value::Null;
    state.members.push(null);
    let board = MemoryBoard::with(state);
    let service = ConfirmMemberDead::new(board.clone(), SteppingClock::fixed(5.0));
    for member in ["zombie", "null", "stranger"] {
        assert_eq!(
            service
                .execute(dead("parent", member, json!("orderly")))
                .unwrap(),
            DeathConfirmation::AlreadyDead,
            "{member}"
        );
    }
    let after = board.snapshot();
    assert!(after.events.is_empty());
    assert_eq!(after.members[3].text("status"), Some("zombie"));
}

/// Served over another repository, the confirmation writes to that board
/// alone.
#[test]
fn over_confirms_on_the_given_board() {
    let composed_over = MemoryBoard::with(board());
    let other = MemoryBoard::with(board());
    ConfirmMemberDead::new(composed_over.clone(), SteppingClock::fixed(5.0))
        .over(other.clone())
        .execute(dead("parent", "worker", json!("orderly")))
        .unwrap();
    assert_eq!(other.snapshot().members[1].text("status"), Some("dead"));
    assert!(composed_over.transactions().is_empty());
}
