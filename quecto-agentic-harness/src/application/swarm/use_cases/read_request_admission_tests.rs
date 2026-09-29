use serde_json::json;

use super::ReadRequestAdmission;
use crate::application::swarm::board_test_support::{
    MemoryBoard, SteppingClock, member_row, paused, running_board, usage,
};
use crate::application::swarm::dto::BudgetEffect;
use crate::domain::swarm::{BoardError, RefusalKind};

/// Any member, a dead one included, reads the run and its members; a
/// budget that allows changes nothing.
#[test]
fn the_admission_reads_the_run_and_its_members() {
    let mut state = running_board(100.0);
    state.members.push(member_row("gone", "dead"));
    let board = MemoryBoard::with(state);
    let service = ReadRequestAdmission::new(board.clone(), SteppingClock::fixed(50.0));
    let admission = service.execute("gone").unwrap();
    assert_eq!(admission.effect, BudgetEffect::Unchanged);
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

/// An exhausted budget pauses the running run before it is read, so the
/// admission sees the pause it made.
#[test]
fn an_exhausted_budget_pauses_the_run_the_admission_reads() {
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
    assert_eq!(admission.effect, BudgetEffect::Paused);
    assert_eq!(admission.view.status.as_deref(), Some("paused"));
    assert_eq!(admission.view.outcome.as_deref(), Some("budget-exhausted"));
    assert_eq!(admission.view.control_generation, 2, "the pause event");
    assert_eq!(
        board.journal(),
        [
            "propose_outcome budget-exhausted",
            "event stop",
            "event paused"
        ],
        "already warned: no second warning"
    );
}

/// A paused run is not paused again; a budget that has not warned yet
/// still warns once.
#[test]
fn a_paused_run_only_warns() {
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
    assert_eq!(admission.effect, BudgetEffect::Warned);
    assert_eq!(
        board.journal(),
        [
            r#"configure_usage_budget {"token_limit":100,"strict_unknown":false,"warned":true}"#,
            "event usage-warning",
        ]
    );
}
