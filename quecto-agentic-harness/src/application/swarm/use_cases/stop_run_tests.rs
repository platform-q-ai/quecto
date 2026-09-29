use serde_json::{Value, json};

use super::StopRun;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, SteppingClock, paused, running_board,
};
use crate::application::swarm::dto::{RunTransition, StopRunRequest};
use crate::domain::swarm::{BoardError, RunState};

fn stop(status: Value, reason: &str) -> StopRunRequest {
    StopRunRequest {
        actor: "parent".to_owned(),
        status,
        reason: Value::from(reason),
    }
}

fn with_status(status: &'static str) -> BoardState {
    let mut state = running_board(100.0);
    state.run.as_mut().unwrap().record.status = Some(RunState::new(status));
    state
}

/// Only the `STOP_STATUSES` are accepted, and the reason is bounded
/// first, before any transaction.
#[test]
fn the_status_and_reason_are_checked_before_the_store() {
    let board = MemoryBoard::with(running_board(100.0));
    let service = StopRun::new(board.clone(), SteppingClock::fixed(50.0));
    assert_eq!(
        service.execute(stop(json!("blocked"), " ")).unwrap_err(),
        BoardError::new("stop reason must be nonempty and at most 8192 bytes")
    );
    for status in [
        json!("succeeded"),
        json!("paused"),
        json!(null),
        json!(["blocked"]),
    ] {
        assert_eq!(
            service.execute(stop(status, "why")).unwrap_err(),
            BoardError::new("invalid non-success outcome")
        );
    }
    assert!(board.transactions().is_empty(), "refused before the store");
}

/// `test_coordinator_stop_is_a_resumable_pause_only_the_supervisor_lifts`:
/// a running run pauses holding the outcome (`stop` then `paused`); the
/// same stop again is a no-op; another verdict is refused naming it.
#[test]
fn a_stop_ends_a_running_run_as_a_pause_holding_the_outcome() {
    let board = MemoryBoard::with(running_board(100.0));
    let service = StopRun::new(board.clone(), SteppingClock::fixed(50.0));
    let answer = service
        .execute(stop(json!("blocked"), "needs input"))
        .unwrap();
    assert_eq!(answer.transition, RunTransition::Applied);
    assert_eq!(
        (
            answer.receipt.status.as_deref(),
            answer.receipt.outcome.as_deref()
        ),
        (Some("paused"), Some("blocked"))
    );
    let actions: Vec<_> = board
        .snapshot()
        .events
        .iter()
        .map(|event| (event.action.clone(), event.detail.clone()))
        .collect();
    assert_eq!(
        actions,
        [
            (
                "stop".to_owned(),
                json!({"status": "blocked", "reason": "needs input"})
            ),
            (
                "paused".to_owned(),
                json!({"reason": "needs input", "started": 50.0, "outcome": "blocked"})
            ),
        ]
    );
    let again = service.execute(stop(json!("blocked"), "other")).unwrap();
    assert_eq!(again.transition, RunTransition::Unchanged);
    assert_eq!(
        service.execute(stop(json!("failed"), "x")).unwrap_err(),
        BoardError::new(
            "run already paused (blocked: needs input); only the supervisor can resume or \
             close it, and op=cancel_run cancels it"
        )
    );
}

/// `test_cancellation_stays_terminal_even_while_ended`: cancellation drops
/// a held outcome and is terminal at once; a cancelled run stays as it
/// is; a setup placeholder has nothing to cancel; a closed run is not
/// cancelled over.
#[test]
fn cancellation_is_terminal_and_refused_before_and_after_a_run() {
    let held = paused(running_board(100.0), 40.0, Some(("failed", "broken")));
    let board = MemoryBoard::with(held);
    let service = StopRun::new(board.clone(), SteppingClock::fixed(50.0));
    let answer = service
        .execute(stop(json!("cancelled"), "gave up"))
        .unwrap();
    assert_eq!(answer.transition, RunTransition::Applied);
    let run = board.snapshot().run.unwrap().record;
    assert_eq!(
        (run.status, run.outcome, run.outcome_reason),
        (Some(RunState::CANCELLED), None, None)
    );
    assert_eq!(
        board.snapshot().events.last().unwrap().detail,
        json!({"status": "cancelled", "reason": "gave up"})
    );
    let again = service.execute(stop(json!("cancelled"), "again")).unwrap();
    assert_eq!(again.transition, RunTransition::Unchanged);
    for (status, refusal) in [
        (
            "setup",
            "run not created yet; nothing to cancel. To start one: swarm op=create",
        ),
        ("succeeded", "run already succeeded"),
    ] {
        let service = StopRun::new(
            MemoryBoard::with(with_status(status)),
            SteppingClock::fixed(50.0),
        );
        assert_eq!(
            service
                .execute(stop(json!("cancelled"), "late"))
                .unwrap_err(),
            BoardError::new(refusal)
        );
    }
}
