use serde_json::{Value, json};

use super::UnblockTask;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, SteppingClock, member_row, running_board, stored_task,
};
use crate::application::swarm::dto::{TaskTransition, UnblockTaskRequest};
use crate::domain::swarm::{BoardError, RefusalKind};

/// Task 1 blocked, task 2 claimed and task 3 submitted, all by the worker
/// under `stored-token`; task 4 blocked by the parent.
fn board() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    let mut blocked = stored_task(1, "blocked", json!([]), Some("worker"));
    blocked.set("blocker", json!("awaiting approval"));
    state.tasks = vec![
        blocked,
        stored_task(2, "claimed", json!([]), Some("worker")),
        stored_task(3, "submitted", json!([]), Some("worker")),
        stored_task(4, "blocked", json!([]), Some("parent")),
    ];
    state
}

fn unblock(task_id: Value, token: &str, reason: Value) -> UnblockTaskRequest {
    UnblockTaskRequest {
        actor: "worker".to_owned(),
        task_id,
        token: Value::from(token),
        reason,
    }
}

/// The resolved blocker resumes the original claim: the task is `claimed`
/// under the same token, without blocker, and `unblocked{task,reason}` is
/// recorded; a claimed task resumes as a no-op.
#[test]
fn a_resolved_blocker_resumes_the_original_claim() {
    let board = MemoryBoard::with(board());
    let service = UnblockTask::new(board.clone(), SteppingClock::fixed(50.0));
    let change = service
        .execute(unblock(
            json!(true),
            "stored-token",
            json!("approval received"),
        ))
        .unwrap();
    assert_eq!(change.transition, TaskTransition::Applied);
    assert_eq!(
        (change.task.get("id"), change.task.text("status")),
        (Some(&json!(1)), Some("claimed")),
        "the answer is the row as it now stands, its id the one it holds (#2394)"
    );
    for task in [1, 2] {
        assert_eq!(
            service
                .execute(unblock(json!(task), "stored-token", json!("again")))
                .unwrap()
                .transition,
            TaskTransition::Unchanged,
            "{task}"
        );
    }
    let state = board.snapshot();
    let task = &state.tasks[0];
    assert_eq!(task.text("status"), Some("claimed"));
    assert_eq!(task.text("token"), Some("stored-token"));
    assert_eq!(task.get("blocker"), Some(&Value::Null));
    assert_eq!(state.events.len(), 1);
    assert_eq!(
        (state.events[0].action.as_str(), &state.events[0].detail),
        (
            "unblocked",
            &json!({"task": true, "reason": "approval received"})
        )
    );
}

/// The resolution is bounded before the gate; submitted work cannot
/// resume, and only the claim's owner may resume it.
#[test]
fn only_the_owner_resumes_blocked_or_claimed_work() {
    let board = MemoryBoard::with(board());
    let service = UnblockTask::new(board.clone(), SteppingClock::fixed(50.0));
    assert_eq!(
        service
            .execute(unblock(json!(1), "stored-token", json!("")))
            .unwrap_err(),
        BoardError::new(
            RefusalKind::Invalid,
            "resolution must be nonempty and at most 8192 bytes"
        )
    );
    assert!(board.transactions().is_empty());
    for (request, refusal) in [
        (
            unblock(json!(3), "stored-token", json!("r")),
            (
                RefusalKind::WrongState,
                "only blocked or claimed work may resume",
            ),
        ),
        (
            unblock(json!(4), "stored-token", json!("r")),
            (RefusalKind::StaleToken, "stale or unowned claim"),
        ),
        (
            unblock(json!(1), "stale", json!("r")),
            (RefusalKind::StaleToken, "stale or unowned claim"),
        ),
    ] {
        assert_eq!(
            service.execute(request).unwrap_err(),
            BoardError::new(refusal.0, refusal.1)
        );
    }
    let state = board.snapshot();
    assert_eq!(state.tasks[0].text("status"), Some("blocked"));
    assert!(state.events.is_empty());
}
