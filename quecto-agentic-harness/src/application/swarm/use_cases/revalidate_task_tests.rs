use std::sync::Arc;

use serde_json::{Value, json};

use super::RevalidateTask;
use crate::application::swarm::board_test_support::{
    BoardState, CompactEncoding, MemoryBoard, SteppingClock, member_row, running_board, stored_task,
};
use crate::application::swarm::dto::RevalidateTaskRequest;
use crate::domain::swarm::BoardError;

/// A running run with task 1 completed at R1 and task 2 submitted.
fn board_state() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    let mut completed = stored_task(1, "completed", json!([]), Some("worker"));
    completed.set("evidence", json!([{"artifact": "a", "revision": "R1"}]));
    state.tasks = vec![
        completed,
        stored_task(2, "submitted", json!([]), Some("worker")),
    ];
    state
}

fn revalidate(
    actor: &str,
    task_id: Value,
    revision: Value,
    evidence: Value,
) -> RevalidateTaskRequest {
    RevalidateTaskRequest {
        actor: actor.to_owned(),
        task_id,
        revision,
        evidence,
    }
}

fn service(board: &Arc<MemoryBoard>) -> RevalidateTask {
    RevalidateTask::new(
        board.clone(),
        SteppingClock::fixed(50.0),
        Arc::new(CompactEncoding),
    )
}

/// The evidence's encoding is bounded before the store is opened.
#[test]
fn the_encoded_evidence_is_bounded_before_the_store() {
    let board = MemoryBoard::with(board_state());
    let long = json!([{"artifact": "x".repeat(8_200), "revision": "R2"}]);
    assert_eq!(
        service(&board)
            .execute(revalidate("parent", json!(1), json!("R2"), long))
            .unwrap_err(),
        BoardError::new("evidence references must be nonempty and at most 8192 bytes")
    );
    assert!(board.transactions().is_empty(), "refused before the store");
}

/// The coordinator alone revalidates, a known task, completed, with new
/// evidence at the revision; each refusal writes nothing.
#[test]
fn only_the_coordinator_revalidates_completed_work_at_the_revision() {
    let good = json!([{"artifact": "b", "revision": "R2"}]);
    for (actor, task, revision, evidence, message) in [
        (
            "worker",
            json!(1),
            json!("R2"),
            good.clone(),
            "only the designated coordinator may do this",
        ),
        (
            "parent",
            json!(9),
            json!("R2"),
            good.clone(),
            "unknown task",
        ),
        (
            "parent",
            json!(2),
            json!("R2"),
            good.clone(),
            "only completed tasks may be revalidated",
        ),
        (
            "parent",
            json!(1),
            json!(" "),
            good.clone(),
            "new artifact and revision evidence required",
        ),
        (
            "parent",
            json!(1),
            json!("R2"),
            json!([]),
            "new artifact and revision evidence required",
        ),
        (
            "parent",
            json!(1),
            json!("R2"),
            json!([{"artifact": "a", "revision": "R1"}]),
            "new artifact evidence must match the revalidated revision",
        ),
        (
            "parent",
            json!(1),
            json!("R2"),
            json!([{"artifact": " ", "revision": "R2"}]),
            "new artifact evidence must match the revalidated revision",
        ),
    ] {
        let board = MemoryBoard::with(board_state());
        assert_eq!(
            service(&board)
                .execute(revalidate(actor, task, revision, evidence))
                .unwrap_err(),
            BoardError::new(message)
        );
        assert!(board.snapshot().events.is_empty(), "{message}");
        assert_eq!(
            board.snapshot().tasks[0].get("evidence"),
            Some(&json!([{"artifact": "a", "revision": "R1"}]))
        );
    }
}

/// The evidence is replaced as given (extra keys kept), and the event
/// names the task id and revision as the caller gave them, with the
/// evidence it replaced.
#[test]
fn the_evidence_is_replaced_and_the_previous_evidence_recorded() {
    let board = MemoryBoard::with(board_state());
    let evidence = json!([{"revision": "R2", "artifact": "b", "note": 1.5}]);
    service(&board)
        .execute(revalidate(
            "parent",
            json!("1"),
            json!("R2"),
            evidence.clone(),
        ))
        .unwrap();
    let state = board.snapshot();
    assert_eq!(state.tasks[0].get("evidence"), Some(&evidence));
    assert_eq!(state.tasks[0].text("status"), Some("completed"));
    let event = state.events.last().unwrap();
    assert_eq!(
        (event.action.as_str(), &event.detail),
        (
            "revalidated",
            &json!({
                "task": "1",
                "revision": "R2",
                "previous_evidence": [{"artifact": "a", "revision": "R1"}],
                "evidence": evidence,
            })
        )
    );
    assert!(
        board
            .journal()
            .contains(&"replace_task_evidence \"1\"".to_owned()),
        "{:?}",
        board.journal()
    );
}
