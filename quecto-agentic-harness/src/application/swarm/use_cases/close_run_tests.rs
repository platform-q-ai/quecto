use serde_json::json;

use super::CloseRun;
use crate::application::swarm::board_test_support::{
    MemoryBoard, SteppingClock, member_row, paused, running_board,
};
use crate::application::swarm::dto::RunTransition;
use crate::domain::swarm::BoardError;

/// `test_completion_holds_success_until_the_supervisor_closes_it` (the
/// close half): a running run, or a plain pause, holds nothing to close.
#[test]
fn only_a_held_proposed_outcome_closes() {
    for (state, status) in [
        (running_board(100.0), "running"),
        (paused(running_board(100.0), 40.0, None), "paused"),
        (
            paused(running_board(100.0), 40.0, Some(("paused", "odd"))),
            "paused",
        ),
    ] {
        let service = CloseRun::new(MemoryBoard::with(state), SteppingClock::fixed(50.0));
        assert_eq!(
            service.execute("parent").unwrap_err(),
            BoardError::new(format!(
                "run is {status} without a proposed outcome; resume it or cancel the run"
            ))
        );
    }
    let mut unknown = running_board(100.0);
    unknown.run.as_mut().unwrap().record.status = None;
    let service = CloseRun::new(MemoryBoard::with(unknown), SteppingClock::fixed(50.0));
    assert_eq!(
        service.execute("parent").unwrap_err(),
        BoardError::new("run is None without a proposed outcome; resume it or cancel the run")
    );
}

/// The held outcome becomes the run's status, the outcome and its reason
/// stay, and `closed{status,reason}` records them.
#[test]
fn the_held_outcome_becomes_terminal() {
    let mut held = paused(running_board(100.0), 40.0, Some(("succeeded", "done")));
    held.members.push(member_row("worker", "live"));
    let board = MemoryBoard::with(held);
    let service = CloseRun::new(board.clone(), SteppingClock::fixed(50.0));
    assert_eq!(
        service.execute("worker").unwrap_err(),
        BoardError::new("only the designated coordinator may do this")
    );
    let answer = service.execute("parent").unwrap();
    assert_eq!(answer.transition, RunTransition::Applied);
    assert_eq!(
        (
            answer.receipt.status.as_deref(),
            answer.receipt.outcome.as_deref(),
            answer.receipt.reason.as_deref()
        ),
        (Some("succeeded"), Some("succeeded"), Some("done"))
    );
    let closed = board.snapshot().events.last().cloned().unwrap();
    assert_eq!(
        (closed.action.as_str(), closed.detail),
        ("closed", json!({"status": "succeeded", "reason": "done"}))
    );
}
