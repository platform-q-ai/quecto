use serde_json::json;

use super::LoseCoordinator;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, SteppingClock, member_row, paused, running_board,
};
use crate::application::swarm::dto::LoseCoordinatorRequest;
use crate::application::swarm::use_cases::OverRepository;
use crate::domain::swarm::RunState;

fn lose() -> LoseCoordinatorRequest {
    LoseCoordinatorRequest {
        actor: "parent".to_owned(),
    }
}

fn with_status(status: RunState) -> BoardState {
    let mut state = running_board(1_000.0);
    state.run.as_mut().unwrap().record.status = Some(status);
    state
}

/// A running run is lost as a lost harness: `scope_unknown` naming the
/// caller, then the end by loss; the answer is the run after it.
#[test]
fn a_running_run_is_lost_as_a_failed_pause() {
    let board = MemoryBoard::with(running_board(1_000.0));
    let loss = LoseCoordinator::new(board.clone(), SteppingClock::fixed(5.0))
        .execute(lose())
        .unwrap();
    assert!(loss.lost);
    assert_eq!(loss.found, Some(RunState::RUNNING), "found before the loss");
    assert_eq!(
        (
            loss.run.id.as_deref(),
            loss.run.status.as_deref(),
            loss.run.outcome.as_deref(),
            loss.run.coordinator.as_deref(),
        ),
        (
            Some("run-1"),
            Some("paused"),
            Some("failed"),
            Some("parent")
        )
    );
    assert_eq!(loss.run.deadline, json!(1_000.0));
    let events = board.snapshot().events;
    assert_eq!(
        events[0].detail,
        json!({
            "member": "parent",
            "reason": "harness exited; execution scope unconfirmed; discard environment"
        })
    );
    assert_eq!(events.len(), 3);
}

/// The setup placeholder fails; an outcome-less pause takes `failed`; a
/// held outcome or an ended run is left alone.
#[test]
fn each_run_state_is_lost_or_left_as_python_decides() {
    for (state, lost, status) in [
        (with_status(RunState::SETUP), true, "failed"),
        (paused(running_board(1_000.0), 3.0, None), true, "paused"),
        (
            paused(running_board(1_000.0), 3.0, Some(("blocked", "why"))),
            false,
            "paused",
        ),
        (with_status(RunState::CANCELLED), false, "cancelled"),
        (with_status(RunState::SUCCEEDED), false, "succeeded"),
    ] {
        let events = state.events.len();
        let board = MemoryBoard::with(state);
        let loss = LoseCoordinator::new(board.clone(), SteppingClock::fixed(5.0))
            .execute(lose())
            .unwrap();
        assert_eq!(
            (loss.lost, loss.run.status.as_deref()),
            (lost, Some(status))
        );
        assert_eq!(board.snapshot().events.len() > events, lost, "{status}");
    }
}

/// A caller whose death is confirmed is refused by the gate; served over
/// another repository, the op writes to that board alone.
#[test]
fn the_gate_refuses_a_dead_caller_and_over_serves_the_given_board() {
    let mut state = running_board(1_000.0);
    state.members[0] = member_row("parent", "dead");
    let board = MemoryBoard::with(state);
    let service = LoseCoordinator::new(board.clone(), SteppingClock::fixed(5.0));
    assert!(service.execute(lose()).is_err());
    let other = MemoryBoard::with(running_board(1_000.0));
    assert!(service.over(other.clone()).execute(lose()).unwrap().lost);
    assert!(board.snapshot().events.is_empty());
}
