use serde_json::{Value, json};

use super::RecoverTask;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, SteppingClock, StoredFile, member_row, running_board, stored_task,
};
use crate::application::swarm::dto::{RecoverTaskRequest, Recovered};

/// Task 1 blocked under the dead worker's claim, holding one reservation;
/// task 2 claimed by the live `other`; task 3 ready.
fn board() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "dead"));
    state.members.push(member_row("other", "live"));
    let mut abandoned = stored_task(1, "blocked", json!([]), Some("worker"));
    abandoned.set("evidence", json!([{"artifact": "a", "revision": "r"}]));
    abandoned.set("blocker", json!("worker death confirmed"));
    state.tasks = vec![
        abandoned,
        stored_task(2, "claimed", json!([]), Some("other")),
        stored_task(3, "ready", json!([]), None),
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

fn recover(actor: &str, task_id: Value, release_files: Value) -> RecoverTaskRequest {
    RecoverTaskRequest {
        actor: actor.to_owned(),
        task_id,
        release_files,
    }
}

/// Only the coordinator recovers, only held work, only from a dead owner,
/// and retained reservations only with `release_files` exactly `true`.
#[test]
fn recovery_is_refused_in_pythons_order() {
    let board = MemoryBoard::with(board());
    let service = RecoverTask::new(board.clone(), SteppingClock::fixed(50.0));
    for (actor, task_id, release_files, message) in [
        (
            "other",
            json!(1),
            json!(true),
            "only the designated coordinator may do this",
        ),
        (
            "parent",
            json!(3),
            json!(true),
            "only abandoned active work can be recovered",
        ),
        (
            "parent",
            json!(2),
            json!(true),
            "recovery requires confirmed worker death; revoke(id, reason) reassigns a live owner",
        ),
        (
            "parent",
            json!(1),
            json!(false),
            "reservations retained after an abrupt exit; recover(id, release_files=True) frees them, or revoke(id, reason)",
        ),
        (
            "parent",
            json!(1),
            json!(1),
            "reservations retained after an abrupt exit; recover(id, release_files=True) frees them, or revoke(id, reason)",
        ),
    ] {
        assert_eq!(
            service
                .execute(recover(actor, task_id, release_files))
                .unwrap_err()
                .0,
            message
        );
    }
    let state = board.snapshot();
    assert_eq!(state.files.len(), 1);
    assert!(state.events.is_empty());
}

/// Recovery reopens the task without owner, token, blocker or evidence,
/// frees its reservations and records how many.
#[test]
fn recovery_reopens_the_task_and_frees_its_files() {
    let board = MemoryBoard::with(board());
    let service = RecoverTask::new(board.clone(), SteppingClock::fixed(50.0));
    assert_eq!(
        service
            .execute(recover("parent", json!("1"), json!(true)))
            .unwrap(),
        Recovered {
            reservations_released: 1
        }
    );
    let state = board.snapshot();
    let task = &state.tasks[0];
    assert_eq!(task.text("status"), Some("ready"));
    for column in ["owner", "token", "blocker"] {
        assert_eq!(task.get(column), Some(&Value::Null), "{column}");
    }
    assert_eq!(task.get("evidence"), Some(&json!([])));
    assert!(state.files.is_empty());
    assert_eq!(
        (state.events[0].action.as_str(), &state.events[0].detail),
        (
            "recovered",
            &json!({"task": "1", "reservations_released": 1})
        )
    );
}

/// Without reservations, `release_files` is not needed.
#[test]
fn recovery_without_reservations_needs_no_release() {
    let mut state = board();
    state.files.clear();
    let board = MemoryBoard::with(state);
    let recovered = RecoverTask::new(board.clone(), SteppingClock::fixed(50.0))
        .execute(recover("parent", json!(1), json!(false)))
        .unwrap();
    assert_eq!(recovered.reservations_released, 0);
    assert_eq!(board.snapshot().tasks[0].text("status"), Some("ready"));
}
