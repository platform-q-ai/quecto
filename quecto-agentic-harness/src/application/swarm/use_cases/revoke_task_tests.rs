use std::sync::Arc;

use serde_json::{Value, json};

use super::RevokeTask;
use crate::application::swarm::board_test_support::{
    BoardState, CompactEncoding, MemoryBoard, SteppingClock, StoredFile, StoredMessage, member_row,
    running_board, stored_task,
};
use crate::application::swarm::dto::{Revocation, RevokeTaskRequest};
use crate::domain::swarm::BoardError;

/// Task 1 submitted by the live worker with a reservation; task 2 ready;
/// task 3 completed; task 4 ready but waiting on task 2 (it reads
/// blocked).
fn board() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    let mut submitted = stored_task(1, "submitted", json!([]), Some("worker"));
    submitted.set("evidence", json!([{"artifact": "a", "revision": "r"}]));
    state.tasks = vec![
        submitted,
        stored_task(2, "ready", json!([]), None),
        stored_task(3, "completed", json!([]), Some("worker")),
        stored_task(4, "ready", json!([2]), None),
    ];
    state.files = vec![StoredFile {
        path: "src/a.rs".to_owned(),
        task: 1,
        owner: "worker".to_owned(),
        claim: "stored-token".to_owned(),
        token: "r".to_owned(),
    }];
    state
}

fn service(board: &Arc<MemoryBoard>) -> RevokeTask {
    RevokeTask::new(
        board.clone(),
        SteppingClock::fixed(50.0),
        Arc::new(CompactEncoding),
    )
}

fn revoke(actor: &str, task_id: Value, reason: Value) -> RevokeTaskRequest {
    RevokeTaskRequest {
        actor: actor.to_owned(),
        task_id,
        reason,
    }
}

/// The coordinator takes the claim back: the task is ready without owner,
/// token, blocker or evidence, its reservations go, the audit names the
/// reason and previous owner, and the previous owner is told.
#[test]
fn revocation_reopens_the_task_and_tells_the_previous_owner() {
    let board = MemoryBoard::with(board());
    let revoked = service(&board)
        .execute(revoke("parent", json!(1), json!("member suspended")))
        .unwrap();
    assert_eq!(revoked.revocation, Revocation::Revoked { notified: true });
    let task = revoked.task;
    assert_eq!(task.text("status"), Some("ready"));
    for column in ["owner", "token", "blocker"] {
        assert_eq!(task.get(column), Some(&Value::Null), "{column}");
    }
    assert_eq!(task.get("evidence"), Some(&json!([])));
    let state = board.snapshot();
    assert!(state.files.is_empty());
    assert_eq!(
        state.messages,
        [StoredMessage {
            id: 1,
            sender: "parent".to_owned(),
            recipient: json!("worker"),
            body: "claim on task 1 revoked by the coordinator: member suspended".to_owned(),
            status: "accepted".to_owned(),
        }]
    );
    let events: Vec<(&str, &Value)> = state
        .events
        .iter()
        .map(|e| (e.action.as_str(), &e.detail))
        .collect();
    assert_eq!(
        events,
        [
            (
                "revoked",
                &json!({"task": 1, "reason": "member suspended", "previous_owner": "worker"})
            ),
            (
                "message_accepted",
                &json!({"message": 1, "recipient": "worker", "revision": null})
            ),
        ]
    );
}

/// An unowned ready (or dependency-blocked) task is answered as it stands;
/// completed work is refused; the reason is bounded before the gate; only
/// the coordinator revokes.
#[test]
fn revocation_refuses_or_leaves_what_it_cannot_take() {
    let board = MemoryBoard::with(board());
    let service = service(&board);
    for task_id in [2, 4] {
        let unowned = service
            .execute(revoke("parent", json!(task_id), json!("why")))
            .unwrap();
        assert_eq!(unowned.revocation, Revocation::Unowned);
        assert_eq!(unowned.task.get("id"), Some(&json!(task_id)));
    }
    for (actor, task_id, reason, message) in [
        (
            "parent",
            json!(3),
            json!("late"),
            "only claimed, blocked or submitted work can be revoked",
        ),
        (
            "parent",
            json!(1),
            json!(""),
            "revocation reason must be nonempty and at most 8192 bytes",
        ),
        (
            "worker",
            json!(1),
            json!("mine"),
            "only the designated coordinator may do this",
        ),
    ] {
        assert_eq!(
            service.execute(revoke(actor, task_id, reason)).unwrap_err(),
            BoardError::new(message)
        );
    }
    let state = board.snapshot();
    assert!(state.events.is_empty());
    assert_eq!(state.files.len(), 1);
}

/// The message names the task id as Python's `str()` writes it.
#[test]
fn the_message_names_the_task_id_as_python_writes_it() {
    for (task_id, written) in [
        (json!(true), "True"),
        (json!("1"), "1"),
        (json!(1.0), "1.0"),
    ] {
        let board = MemoryBoard::with(board());
        service(&board)
            .execute(revoke("parent", task_id, json!("r")))
            .unwrap();
        assert_eq!(
            board.snapshot().messages[0].body,
            format!("claim on task {written} revoked by the coordinator: r")
        );
    }
}
