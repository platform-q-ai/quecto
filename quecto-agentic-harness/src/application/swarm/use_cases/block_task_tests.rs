use serde_json::{Value, json};

use super::BlockTask;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, SteppingClock, member_row, running_board, stored_task,
};
use crate::application::swarm::dto::{BlockTaskRequest, TaskTransition};
use crate::domain::swarm::{BoardError, RefusalKind};

/// Task 1 claimed by the worker (token `stored-token`), task 2 submitted
/// by the worker, task 3 claimed by the parent.
fn board() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    state.tasks = vec![
        stored_task(1, "claimed", json!([]), Some("worker")),
        stored_task(2, "submitted", json!([]), Some("worker")),
        stored_task(3, "claimed", json!([]), Some("parent")),
    ];
    state
}

fn block(task_id: Value, token: &str, reason: Value) -> BlockTaskRequest {
    BlockTaskRequest {
        actor: "worker".to_owned(),
        task_id,
        token: Value::from(token),
        reason,
    }
}

/// `test_resolved_blocker_resumes_original_claim_without_releasing_files`'s
/// first step: the owner blocks its claim with a reason, recorded as
/// `blocked{task,reason}`; the same reason again changes nothing and
/// records nothing; another reason replaces it.
#[test]
fn the_owner_blocks_its_claim_once_per_reason() {
    let board = MemoryBoard::with(board());
    let service = BlockTask::new(board.clone(), SteppingClock::fixed(50.0));
    let reason = json!("awaiting approval");
    let change = service
        .execute(block(json!("1"), "stored-token", reason.clone()))
        .unwrap();
    assert_eq!(change.transition, TaskTransition::Applied);
    assert_eq!(
        (change.task.get("id"), change.task.text("status")),
        (Some(&json!(1)), Some("blocked")),
        "the answer is the row as it now stands, its id the one it holds (#2394)"
    );
    assert_eq!(change.task.get("blocker"), Some(&reason));
    assert_eq!(
        service
            .execute(block(json!(1), "stored-token", reason.clone()))
            .unwrap()
            .transition,
        TaskTransition::Unchanged
    );
    assert_eq!(
        service
            .execute(block(json!(1), "stored-token", json!("another")))
            .unwrap()
            .transition,
        TaskTransition::Applied
    );
    let state = board.snapshot();
    assert_eq!(state.tasks[0].text("status"), Some("blocked"));
    assert_eq!(state.tasks[0].text("blocker"), Some("another"));
    assert_eq!(state.tasks[0].text("token"), Some("stored-token"));
    let events: Vec<(&str, &Value)> = state
        .events
        .iter()
        .map(|event| (event.action.as_str(), &event.detail))
        .collect();
    assert_eq!(
        events,
        [
            (
                "blocked",
                &json!({"task": "1", "reason": "awaiting approval"})
            ),
            ("blocked", &json!({"task": 1, "reason": "another"})),
        ]
    );
}

/// The reason is bounded before the gate; a stale or foreign claim and
/// submitted work are refused, and nothing changes.
#[test]
fn blockers_are_refused_for_bad_reasons_stale_claims_and_submitted_work() {
    let board = MemoryBoard::with(board());
    let service = BlockTask::new(board.clone(), SteppingClock::fixed(50.0));
    let bound = (
        RefusalKind::Invalid,
        "blocker must be nonempty and at most 8192 bytes",
    );
    for (request, refusal) in [
        (block(json!(1), "stored-token", json!(" ")), bound),
        (block(json!(1), "stored-token", json!(5)), bound),
        (
            block(json!(1), "stored-token", json!("x".repeat(8193))),
            bound,
        ),
    ] {
        assert_eq!(
            service.execute(request).unwrap_err(),
            BoardError::new(refusal.0, refusal.1)
        );
    }
    assert!(
        board.transactions().is_empty(),
        "argument checks come first"
    );
    for (request, refusal) in [
        (
            block(json!(1), "stale", json!("r")),
            (RefusalKind::StaleToken, "stale or unowned claim"),
        ),
        (
            block(json!(3), "stored-token", json!("r")),
            (RefusalKind::StaleToken, "stale or unowned claim"),
        ),
        (
            block(json!(9), "stored-token", json!("r")),
            (RefusalKind::NotFound, "unknown task"),
        ),
        (
            block(json!(2), "stored-token", json!("r")),
            (
                RefusalKind::Immutable,
                "submitted evidence is immutable; release and reclaim before revising",
            ),
        ),
    ] {
        assert_eq!(
            service.execute(request).unwrap_err(),
            BoardError::new(refusal.0, refusal.1)
        );
    }
    service
        .execute(block(json!(1), "stored-token", json!("x".repeat(8192))))
        .unwrap();
    let state = board.snapshot();
    assert_eq!(state.tasks[1].text("status"), Some("submitted"));
    assert_eq!(state.events.len(), 1);
}
