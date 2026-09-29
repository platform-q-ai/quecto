use serde_json::json;

use super::apply_usage_budget;
use crate::application::swarm::board_operation::atomic;
use crate::application::swarm::board_test_support::{
    MemoryBoard, SteppingClock, paused, running_board, usage,
};
use crate::application::swarm::dto::BudgetEffect;
use crate::domain::swarm::BoardError;

fn applied(board: &MemoryBoard) -> Result<BudgetEffect, BoardError> {
    let clock = SteppingClock::fixed(50.0);
    atomic(board, false, |transaction| {
        apply_usage_budget(transaction, &*clock, "worker")
    })
}

/// A budget that allows writes nothing and never reads `warned`, so a
/// budget without it is taken as Python takes it.
#[test]
fn an_allowing_budget_changes_nothing() {
    let mut state = running_board(100.0);
    state.usage = Some(usage(json!({"token_limit": null}), 500, 3));
    let board = MemoryBoard::with(state);
    assert_eq!(applied(&board).unwrap(), BudgetEffect::Unchanged);
    let mut state = running_board(100.0);
    state.usage = Some(usage(
        json!({"token_limit": 100, "strict_unknown": false}),
        10,
        0,
    ));
    let board = MemoryBoard::with(state);
    assert_eq!(applied(&board).unwrap(), BudgetEffect::Unchanged);
    assert!(board.journal().is_empty());
}

/// The warning keeps the stored key order, sets only `warned`, and records
/// the decision with the totals as stored; a budget that already warned
/// does not warn again.
#[test]
fn the_warning_is_written_once_in_the_stored_key_order() {
    let mut state = running_board(100.0);
    state.usage = Some(usage(
        json!({"warned": false, "strict_unknown": false, "token_limit": 100, "note": "kept"}),
        85,
        0,
    ));
    let board = MemoryBoard::with(state);
    assert_eq!(applied(&board).unwrap(), BudgetEffect::Warned);
    assert_eq!(
        board.journal(),
        [
            r#"configure_usage_budget {"warned":true,"strict_unknown":false,"token_limit":100,"note":"kept"}"#,
            "event usage-warning",
        ]
    );
    let event = board.snapshot().events.pop().unwrap();
    assert_eq!(event.actor, "worker");
    assert_eq!(
        event.detail,
        json!({"decision": "warn", "observed_tokens": 85, "token_limit": 100,
               "unknown_usage_requests": 0})
    );
    assert_eq!(applied(&board).unwrap(), BudgetEffect::Unchanged);
    assert_eq!(board.journal().len(), 2, "no second warning");
}

/// A pause of a paused run only warns; a budget that must warn but holds
/// no `warned` is an edited record.
#[test]
fn a_paused_run_is_not_ended_again_and_an_edited_budget_is_refused() {
    let mut state = paused(running_board(100.0), 10.0, None);
    state.usage = Some(usage(
        json!({"token_limit": 5, "strict_unknown": false, "warned": true}),
        5,
        0,
    ));
    let board = MemoryBoard::with(state);
    assert_eq!(applied(&board).unwrap(), BudgetEffect::Unchanged);
    assert!(board.journal().is_empty());
    let mut state = running_board(100.0);
    state.usage = Some(usage(
        json!({"token_limit": 5, "strict_unknown": false}),
        5,
        0,
    ));
    let board = MemoryBoard::with(state);
    assert_eq!(
        applied(&board).unwrap_err(),
        BoardError::new("the board's usage budget is not as the board writes it")
    );
}
