use serde_json::{Value, json};

use super::{BUDGET_ARGUMENTS, ConfigureUsageBudget};
use crate::application::swarm::board_test_support::{
    MemoryBoard, SteppingClock, member_row, running_board, usage,
};
use crate::application::swarm::dto::{BudgetChange, BudgetEffect, ConfigureUsageBudgetRequest};
use crate::domain::swarm::{BoardError, RefusalKind, RunState};

fn budget(actor: &str, token_limit: Value, strict_unknown: Value) -> ConfigureUsageBudgetRequest {
    ConfigureUsageBudgetRequest {
        actor: actor.to_owned(),
        token_limit,
        strict_unknown,
    }
}

/// The limit is `None` or an int of 1 through `2**63 - 1` (never a bool
/// or a float), the strictness a bool, checked before any transaction.
#[test]
fn the_arguments_are_checked_before_the_store() {
    let board = MemoryBoard::with(running_board(100.0));
    let service = ConfigureUsageBudget::new(board.clone(), SteppingClock::fixed(50.0));
    for (limit, strict) in [
        (json!(0), json!(true)),
        (json!(-1), json!(true)),
        (json!(9_223_372_036_854_775_808_u64), json!(true)),
        (json!(1.0), json!(true)),
        (json!(true), json!(true)),
        (json!("5"), json!(true)),
        (json!(5), json!(1)),
        (json!(5), json!(null)),
        (json!(null), json!("yes")),
    ] {
        assert_eq!(
            service
                .execute(budget("parent", limit, strict))
                .unwrap_err(),
            BoardError::new(RefusalKind::Invalid, BUDGET_ARGUMENTS)
        );
    }
    assert!(board.transactions().is_empty(), "refused before the store");
}

/// Only the coordinator sets the budget.
#[test]
fn only_the_coordinator_configures_the_budget() {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    let board = MemoryBoard::with(state);
    let service = ConfigureUsageBudget::new(board.clone(), SteppingClock::fixed(50.0));
    assert_eq!(
        service
            .execute(budget("worker", json!(200), json!(true)))
            .unwrap_err(),
        BoardError::new(
            RefusalKind::NotCoordinator,
            "only the designated coordinator may do this"
        )
    );
    assert!(board.journal().is_empty());
}

/// A new budget is written in Python's insertion order, `warned` false,
/// with the event `usage-budget`; the same budget again writes nothing.
#[test]
fn a_new_budget_is_written_once_and_the_same_budget_is_left_as_it_is() {
    let board = MemoryBoard::with(running_board(100.0));
    let service = ConfigureUsageBudget::new(board.clone(), SteppingClock::fixed(50.0));
    let configured = service
        .execute(budget("parent", json!(200), json!(true)))
        .unwrap();
    assert_eq!(configured.change, BudgetChange::Configured);
    assert_eq!(configured.effect, BudgetEffect::Unchanged);
    assert_eq!(
        serde_json::to_string(&configured.report.budget).unwrap(),
        r#"{"token_limit":200,"strict_unknown":true,"warned":false}"#
    );
    assert_eq!(
        board.journal(),
        [
            r#"configure_usage_budget {"token_limit":200,"strict_unknown":true,"warned":false}"#,
            "event usage-budget",
        ]
    );
    let event = board.snapshot().events.pop().unwrap();
    assert_eq!(
        (event.actor.as_str(), event.detail),
        (
            "parent",
            json!({"token_limit": 200, "strict_unknown": true})
        )
    );
    let same = service
        .execute(budget("parent", json!(200), json!(true)))
        .unwrap();
    assert_eq!(same.change, BudgetChange::Unchanged);
    assert_eq!(board.journal().len(), 2, "nothing more is written");
    let default = MemoryBoard::with(running_board(100.0));
    let lifted = ConfigureUsageBudget::new(default.clone(), SteppingClock::fixed(50.0))
        .execute(budget("parent", json!(null), json!(false)))
        .unwrap();
    assert_eq!(
        lifted.change,
        BudgetChange::Unchanged,
        "no budget row reads as no limit, not strict"
    );
    assert!(default.journal().is_empty());
}

/// A budget the observed usage already exceeds warns once and pauses the
/// running run holding `budget-exhausted`.
#[test]
fn a_budget_already_exceeded_warns_and_pauses_the_run() {
    let mut state = running_board(100.0);
    state.usage = Some(usage(
        json!({"token_limit": null, "strict_unknown": false, "warned": false}),
        80,
        0,
    ));
    let board = MemoryBoard::with(state);
    let service = ConfigureUsageBudget::new(board.clone(), SteppingClock::fixed(50.0));
    let configured = service
        .execute(budget("parent", json!(50), json!(false)))
        .unwrap();
    assert_eq!(configured.effect, BudgetEffect::Paused);
    assert_eq!(
        configured.report.budget,
        json!({"token_limit": 50, "strict_unknown": false, "warned": true})
    );
    let state = board.snapshot();
    let run = state.run.unwrap().record;
    assert_eq!(
        (run.status, run.outcome.as_deref()),
        (Some(RunState::PAUSED), Some("budget-exhausted"))
    );
    let actions: Vec<_> = state.events.iter().map(|e| e.action.as_str()).collect();
    assert_eq!(actions, ["usage-budget", "usage-warning", "stop", "paused"]);
    assert_eq!(
        state.events[1].detail,
        json!({"decision": "pause", "observed_tokens": 80, "token_limit": 50,
               "unknown_usage_requests": 0})
    );
}
