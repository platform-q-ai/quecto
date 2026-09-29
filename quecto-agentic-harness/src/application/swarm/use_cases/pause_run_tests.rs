use serde_json::{Value, json};

use super::PauseRun;
use crate::application::swarm::board_test_support::{
    MemoryBoard, SteppingClock, member_row, paused, running_board,
};
use crate::application::swarm::dto::{PauseRunRequest, RunTransition};
use crate::domain::swarm::{BoardError, RunState};

fn pause(actor: &str, reason: Value) -> PauseRunRequest {
    PauseRunRequest {
        actor: actor.to_owned(),
        reason,
    }
}

/// `test_pause_is_durable_freezes_budget_and_rejects_mutation`: only the
/// coordinator pauses; a running run pauses (its outcome untouched) and
/// records `paused{reason,started}` at the clock's reading; pausing a
/// paused run records nothing more.
#[test]
fn the_coordinator_pauses_a_running_run_once() {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    let board = MemoryBoard::with(state);
    let service = PauseRun::new(board.clone(), SteppingClock::fixed(50.0));
    assert_eq!(
        service.execute(pause("worker", json!("hold"))).unwrap_err(),
        BoardError::new("only the designated coordinator may do this")
    );
    let answer = service.execute(pause("parent", json!("hold"))).unwrap();
    assert_eq!(answer.transition, RunTransition::Applied);
    assert_eq!(answer.receipt.status.as_deref(), Some("paused"));
    assert_eq!(answer.receipt.generation, 1);
    let events = board.snapshot().events;
    assert_eq!(events.len(), 1);
    assert_eq!(
        (events[0].action.as_str(), &events[0].detail, events[0].time),
        ("paused", &json!({"reason": "hold", "started": 50.0}), 50.0)
    );
    let again = service.execute(pause("parent", json!("again"))).unwrap();
    assert_eq!(again.transition, RunTransition::Unchanged);
    assert_eq!(
        board.snapshot().events.len(),
        1,
        "a paused run stays as it is"
    );
}

/// The reason is bounded before any transaction; a run that is neither
/// running nor paused is refused as no new work.
#[test]
fn a_bad_reason_or_an_ended_run_is_refused() {
    let board = MemoryBoard::with(running_board(100.0));
    let service = PauseRun::new(board.clone(), SteppingClock::fixed(50.0));
    for reason in [json!(""), json!(5), json!(" \n")] {
        assert_eq!(
            service.execute(pause("parent", reason)).unwrap_err(),
            BoardError::new("pause reason must be nonempty and at most 8192 bytes")
        );
    }
    assert!(board.transactions().is_empty(), "refused before the store");
    let mut cancelled = running_board(100.0);
    cancelled.run.as_mut().unwrap().record.status = Some(RunState::CANCELLED);
    let service = PauseRun::new(MemoryBoard::with(cancelled), SteppingClock::fixed(50.0));
    assert_eq!(
        service.execute(pause("parent", json!("hold"))).unwrap_err(),
        BoardError::new("run is cancelled; no new work permitted")
    );
    let held = paused(running_board(100.0), 40.0, Some(("blocked", "why")));
    let service = PauseRun::new(MemoryBoard::with(held), SteppingClock::fixed(50.0));
    let answer = service.execute(pause("parent", json!("hold"))).unwrap();
    assert_eq!(
        (answer.transition, answer.receipt.outcome.as_deref()),
        (RunTransition::Unchanged, Some("blocked"))
    );
}
