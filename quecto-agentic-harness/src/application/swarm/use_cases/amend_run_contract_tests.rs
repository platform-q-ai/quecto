use std::sync::Arc;

use serde_json::{Value, json};

use super::AmendRunContract;
use crate::application::swarm::board_test_support::{
    BoardState, CompactEncoding, MemoryBoard, SteppingClock, accepted, member_row, running_board,
};
use crate::application::swarm::dto::AmendRunContractRequest;
use crate::domain::swarm::{BoardError, RefusalKind, RunState};

fn board_state() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    let contract = &mut state.run.as_mut().unwrap().contract;
    contract.goal = "ship feature".to_owned();
    contract.constraints = json!(["clean architecture"]);
    contract.criteria = json!([{"id": "tests", "kind": "command", "description": "pass"}]);
    state.evidence = vec![accepted("parent", "tests", "R1", "command")];
    state
}

fn criteria() -> Value {
    json!([{"id": "tests", "kind": "command", "description": "replacement test"}])
}

fn amend(
    actor: &str,
    goal: Value,
    constraints: Value,
    criteria: Value,
    reason: Value,
) -> AmendRunContractRequest {
    AmendRunContractRequest {
        actor: actor.to_owned(),
        goal,
        constraints,
        criteria,
        reason,
    }
}

fn service(board: &Arc<MemoryBoard>) -> AmendRunContract {
    AmendRunContract::new(
        board.clone(),
        SteppingClock::fixed(50.0),
        Arc::new(CompactEncoding),
    )
}

/// The goal, the reason and the encoded constraints are bounded before
/// the store is opened, in that order.
#[test]
fn the_goal_reason_and_constraints_are_bounded_before_the_store() {
    for (goal, constraints, reason, message) in [
        (
            json!(" "),
            json!([]),
            json!(5),
            "goal must be nonempty and at most 8192 bytes",
        ),
        (
            json!("g"),
            json!(["x".repeat(8_200)]),
            json!(null),
            "amendment reason must be nonempty and at most 8192 bytes",
        ),
        (
            json!("g"),
            json!(["x".repeat(8_200)]),
            json!("r"),
            "constraints must be nonempty and at most 8192 bytes",
        ),
    ] {
        let board = MemoryBoard::with(board_state());
        assert_eq!(
            service(&board)
                .execute(amend(
                    "parent",
                    goal,
                    constraints,
                    json!("not criteria"),
                    reason
                ))
                .unwrap_err(),
            BoardError::new(RefusalKind::Invalid, message)
        );
        assert!(board.transactions().is_empty(), "refused before the store");
    }
}

/// The criteria are checked inside the operation, after authorisation: a
/// worker's amend with bad criteria is refused as not the coordinator's,
/// and the coordinator's with bad criteria changes nothing.
#[test]
fn the_criteria_are_checked_after_authorisation() {
    let board = MemoryBoard::with(board_state());
    assert_eq!(
        service(&board)
            .execute(amend(
                "worker",
                json!("wrong"),
                json!([]),
                json!([]),
                json!("silent change")
            ))
            .unwrap_err(),
        BoardError::new(
            RefusalKind::NotCoordinator,
            "only the designated coordinator may do this"
        )
    );
    for (bad, message) in [
        (json!([]), "explicit evidence criteria required"),
        (
            json!([{"id": "t", "kind": "manual", "description": "d"}]),
            "criteria distinguish command checks from parent-reviewed requirements",
        ),
        (
            json!([
                {"id": "t", "kind": "command", "description": "d"},
                {"id": "t", "kind": "review", "description": "d"}
            ]),
            "duplicate criterion id",
        ),
    ] {
        assert_eq!(
            service(&board)
                .execute(amend("parent", json!("g"), json!([]), bad, json!("r")))
                .unwrap_err(),
            BoardError::new(RefusalKind::Invalid, message)
        );
    }
    let state = board.snapshot();
    assert!(state.events.is_empty());
    assert_eq!(state.evidence.len(), 1, "no evidence deleted");
    assert_eq!(state.run.unwrap().contract.goal, "ship feature");
}

/// `test_amendment_preserves_the_entire_original_contract`: the contract
/// is replaced, every evidence row is deleted, and the event carries the
/// whole contract before and after. Constraints need not be strings: only
/// their encoding is bounded.
#[test]
fn an_amendment_preserves_the_entire_original_contract() {
    let board = MemoryBoard::with(board_state());
    let constraints = json!({"replacement": 1});
    service(&board)
        .execute(amend(
            "parent",
            json!("ship feature"),
            constraints.clone(),
            criteria(),
            json!("approved change"),
        ))
        .unwrap();
    let state = board.snapshot();
    let run = state.run.unwrap();
    assert_eq!(run.contract.constraints, constraints);
    assert_eq!(run.contract.criteria, criteria());
    assert_eq!(run.record.status, Some(RunState::RUNNING));
    assert!(state.evidence.is_empty(), "every evidence row is deleted");
    let event = state.events.last().unwrap();
    assert_eq!(
        (event.actor.as_str(), event.action.as_str()),
        ("parent", "amended")
    );
    assert_eq!(
        event.detail,
        json!({
            "previous_goal": "ship feature",
            "goal": "ship feature",
            "reason": "approved change",
            "before": {
                "goal": "ship feature",
                "constraints": ["clean architecture"],
                "criteria": [{"id": "tests", "kind": "command", "description": "pass"}],
            },
            "after": {"goal": "ship feature", "constraints": constraints, "criteria": criteria()},
        })
    );
    let journal = board.journal();
    let amended = journal
        .iter()
        .position(|entry| entry.starts_with("amend_contract"));
    let deleted = journal
        .iter()
        .position(|entry| entry == "delete_all_evidence");
    assert!(amended < deleted && amended.is_some(), "{journal:?}");
}

/// Amending needs a running run.
#[test]
fn a_paused_run_is_not_amended() {
    let mut state = board_state();
    state.run.as_mut().unwrap().record.status = Some(RunState::PAUSED);
    let board = MemoryBoard::with(state);
    assert_eq!(
        service(&board)
            .execute(amend(
                "parent",
                json!("g"),
                json!([]),
                criteria(),
                json!("r")
            ))
            .unwrap_err(),
        BoardError::new(
            RefusalKind::NotRunning,
            "run is paused; no new work permitted"
        )
    );
}
