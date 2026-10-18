use serde_json::json;

use super::ReadRequestAdmission;
use crate::application::swarm::board_test_support::{
    MemoryBoard, SteppingClock, member_row, paused, running_board, usage,
};
use crate::domain::swarm::{
    events::telemetry::RefusalKind, value_objects::board_error::BoardError,
};

/// Any member, a dead one included, reads the run and its members, and
/// the read changes nothing.
#[test]
fn the_admission_reads_the_run_and_its_members() {
    let mut state = running_board(100.0);
    state.members.push(member_row("gone", "dead"));
    let board = MemoryBoard::with(state);
    let service = ReadRequestAdmission::new(board.clone(), SteppingClock::fixed(50.0));
    let admission = service.execute("gone").unwrap();
    let view = admission.view;
    assert_eq!(
        (view.status.as_deref(), view.coordinator.as_deref()),
        (Some("running"), Some("parent"))
    );
    assert_eq!((view.outcome, view.deadline), (None, 100.0));
    assert_eq!((view.control_generation, view.members.len()), (0, 2));
    assert!(board.journal().is_empty(), "nothing written");
    assert_eq!(
        service.execute("stranger").unwrap_err(),
        BoardError::new(
            RefusalKind::NotMember,
            "invoking member is unknown or death confirmed"
        )
    );
}

/// A budget stored before budgets were dropped (owner decision,
/// 2026-10-14), spent and already warned, is read past: heavy usage never
/// pauses the running run, and the read writes nothing.
#[test]
fn a_stored_spent_budget_never_pauses_the_run_the_admission_reads() {
    let mut state = running_board(100.0);
    state.usage = Some(usage(
        json!({"token_limit": 100, "strict_unknown": true, "warned": true}),
        100,
        0,
    ));
    let board = MemoryBoard::with(state);
    let admission = ReadRequestAdmission::new(board.clone(), SteppingClock::fixed(50.0))
        .execute("parent")
        .unwrap();
    assert_eq!(admission.view.status.as_deref(), Some("running"));
    assert_eq!(admission.view.outcome, None);
    assert_eq!(admission.view.control_generation, 0, "no pause event");
    assert!(board.journal().is_empty(), "nothing written");
}

/// A stored budget near its limit that has not warned never warns: no
/// `warned` is written and no `usage-warning` event.
#[test]
fn a_stored_budget_never_warns() {
    let mut state = paused(running_board(100.0), 10.0, None);
    state.usage = Some(usage(
        json!({"token_limit": 100, "strict_unknown": false, "warned": false}),
        90,
        0,
    ));
    let board = MemoryBoard::with(state);
    let admission = ReadRequestAdmission::new(board.clone(), SteppingClock::fixed(50.0))
        .execute("parent")
        .unwrap();
    assert_eq!(admission.view.status.as_deref(), Some("paused"));
    assert!(board.journal().is_empty(), "nothing written");
}

/// Deadlines stay (owner decision, 2026-10-14): a running run past its
/// deadline is ended first, paused holding `budget-exhausted` with the
/// reason `deadline`, as before; only the token budget's pause is gone.
#[test]
fn a_run_past_its_deadline_still_ends_as_budget_exhausted() {
    let board = MemoryBoard::with(running_board(100.0));
    let admission = ReadRequestAdmission::new(board.clone(), SteppingClock::fixed(150.0))
        .execute("parent")
        .unwrap();
    assert_eq!(admission.view.status.as_deref(), Some("paused"));
    assert_eq!(admission.view.outcome.as_deref(), Some("budget-exhausted"));
    let run = board.snapshot().run.unwrap().record;
    assert_eq!(
        (run.outcome.as_deref(), run.outcome_reason.as_deref()),
        (Some("budget-exhausted"), Some("deadline"))
    );
    assert_eq!(
        board.journal(),
        [
            "propose_outcome budget-exhausted",
            "event stop",
            "event paused"
        ]
    );
}
