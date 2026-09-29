use serde_json::{Value, json};

use super::JoinMember;
use crate::application::swarm::board_test_support::{
    BoardState, CounterIds, MemoryBoard, SteppingClock, running_board, stored_member,
};
use crate::application::swarm::dto::{JoinRunRequest, Joined, LaunchIdentity, RunSummary};
use crate::application::swarm::use_cases::OverRepository;
use crate::domain::swarm::{BoardError, RefusalKind};

/// `worker` live in process 7 started at `s`.
fn board() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(stored_member(
        "worker",
        json!("r"),
        "live",
        [json!(7), json!("s"), Value::Null, json!("parent")],
    ));
    state
}

fn join() -> JoinRunRequest {
    JoinRunRequest {
        member: "worker".to_owned(),
        reservation: json!("r"),
        launch: LaunchIdentity {
            pid: json!(7),
            started: json!("s"),
        },
        socket: Value::Null,
    }
}

fn service(board: &std::sync::Arc<MemoryBoard>) -> JoinMember {
    JoinMember::new(
        board.clone(),
        SteppingClock::fixed(1.0),
        CounterIds::journalling(&board.journal),
    )
}

/// The member's own live process writes nothing and still answers the
/// coordinator's summary.
#[test]
fn the_already_live_branch_answers_the_coordinators_summary() {
    let board = MemoryBoard::with(board());
    let answer = service(&board).execute(join()).unwrap();
    assert_eq!(
        answer.joined,
        Joined::AlreadyLive {
            coordinator: Some("parent".to_owned())
        }
    );
    assert!(matches!(answer.summary, RunSummary::Full(_)));
    assert!(board.snapshot().events.is_empty());
}

/// A coordinator that is nobody (NULL) refuses the closing summary, even
/// where the join wrote nothing.
#[test]
fn a_coordinator_that_is_nobody_refuses_the_summary() {
    let mut state = board();
    state.run.as_mut().unwrap().record.coordinator = None;
    let board = MemoryBoard::with(state);
    assert_eq!(
        service(&board).execute(join()).unwrap_err(),
        BoardError::new(
            RefusalKind::NotMember,
            "invoking member is unknown or death confirmed"
        )
    );
}

#[test]
fn over_joins_on_the_given_board() {
    let composed_over = MemoryBoard::with(board());
    let other = MemoryBoard::with(board());
    service(&composed_over)
        .over(other.clone())
        .execute(join())
        .unwrap();
    assert!(composed_over.transactions().is_empty());
    assert!(!other.transactions().is_empty());
}
