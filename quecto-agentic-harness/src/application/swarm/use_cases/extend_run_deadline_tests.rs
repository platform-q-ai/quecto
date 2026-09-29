use serde_json::{Value, json};

use super::ExtendRunDeadline;
use crate::application::swarm::board_test_support::{
    MemoryBoard, SteppingClock, paused, running_board,
};
use crate::application::swarm::dto::{ExtendRunDeadlineRequest, RunTransition};
use crate::domain::swarm::{BoardError, RunState};

fn extend(seconds: Value) -> ExtendRunDeadlineRequest {
    ExtendRunDeadlineRequest {
        actor: "parent".to_owned(),
        seconds,
    }
}

/// `extend_grants_from_the_later_of_deadline_and_pause_start`: a running
/// run's deadline grows from itself; a paused run's from the later of its
/// deadline and its pause start, since a resume adds the paused interval
/// back. `extended{seconds,deadline}` records the grant.
#[test]
fn extend_grants_from_the_later_of_deadline_and_pause_start() {
    for (state, granted) in [
        (running_board(100.0), 160.0),
        (paused(running_board(100.0), 40.0, None), 160.0),
        (paused(running_board(100.0), 130.0, None), 190.0),
    ] {
        let board = MemoryBoard::with(state);
        let service = ExtendRunDeadline::new(board.clone(), SteppingClock::fixed(50.0));
        let answer = service.execute(extend(json!(60))).unwrap();
        assert_eq!(answer.transition, RunTransition::Applied);
        let state = board.snapshot();
        assert_eq!(state.run.unwrap().record.deadline, granted);
        let extended = state.events.last().unwrap();
        assert_eq!(
            (extended.action.as_str(), &extended.detail),
            ("extended", &json!({"seconds": 60, "deadline": granted}))
        );
    }
}

/// `test_deadline_extension_is_capped_at_seven_days_ahead`: the seconds
/// are an integer of 1 through 604800, checked before any transaction,
/// and the new deadline is at most seven days past the clock.
#[test]
fn extensions_are_bounded_and_capped_at_seven_days_ahead() {
    let board = MemoryBoard::with(running_board(100.0));
    let service = ExtendRunDeadline::new(board.clone(), SteppingClock::fixed(50.0));
    for seconds in [
        json!(0),
        json!(604_801),
        json!(60.0),
        json!(true),
        json!("60"),
    ] {
        assert_eq!(
            service.execute(extend(seconds)).unwrap_err(),
            BoardError::new("deadline extension must be 1..604800 seconds")
        );
    }
    assert!(board.transactions().is_empty(), "refused before the store");
    assert_eq!(
        service.execute(extend(json!(604_800))).unwrap_err(),
        BoardError::new("deadline may be at most seven days ahead, as at creation")
    );
    service.execute(extend(json!(604_750))).unwrap();
    assert_eq!(board.snapshot().run.unwrap().record.deadline, 604_850.0);
    let mut cancelled = running_board(100.0);
    cancelled.run.as_mut().unwrap().record.status = Some(RunState::CANCELLED);
    let service = ExtendRunDeadline::new(MemoryBoard::with(cancelled), SteppingClock::fixed(50.0));
    assert_eq!(
        service.execute(extend(json!(60))).unwrap_err(),
        BoardError::new("run is cancelled; nothing to extend")
    );
}

/// Python reads the clock for the seven-day horizon and again, inside
/// `store.event`, for the event's time (#2318 review nit), after the
/// gate's expiry check.
#[test]
fn the_extension_is_recorded_at_the_reading_after_its_horizon() {
    let board = MemoryBoard::with(running_board(100.0));
    let service = ExtendRunDeadline::new(board.clone(), SteppingClock::new(&[50.0, 51.0, 52.0]));
    service.execute(extend(json!(60))).unwrap();
    let extended = board.snapshot().events.last().unwrap().clone();
    assert_eq!(
        (&extended.detail, extended.time),
        (&json!({"seconds": 60, "deadline": 160.0}), 52.0)
    );
}

/// `outside_edited_control_records` (#2318 review): a pause start only a
/// file edited outside the board holds, one that is not a float, diverges.
/// Python's `max(deadline, started)` keeps an integer start later than the
/// deadline, so its `extended` event records an integer deadline where
/// this records the float; and Python counts a boolean start as 0 or 1
/// where this refuses it naming the record.
#[test]
fn a_pause_start_that_is_not_a_float_diverges() {
    let started = |value: Value| {
        let mut state = paused(running_board(100.0), 0.0, None);
        state.events[0].detail = json!({"reason": "hold", "started": value});
        let board = MemoryBoard::with(state);
        let service = ExtendRunDeadline::new(board.clone(), SteppingClock::fixed(50.0));
        (board, service.execute(extend(json!(60))))
    };
    let (board, answer) = started(json!(130));
    answer.unwrap();
    let extended = board.snapshot().events.last().unwrap().detail.to_string();
    assert_eq!(extended, r#"{"seconds":60,"deadline":190.0}"#);
    let (_, answer) = started(json!(true));
    assert_eq!(
        answer.unwrap_err(),
        BoardError::new("the board's pause record is not as the board writes it")
    );
}
