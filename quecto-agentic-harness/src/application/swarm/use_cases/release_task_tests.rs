use serde_json::{Value, json};

use super::ReleaseTask;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, SteppingClock, StoredFile, member_row, running_board, stored_task,
};
use crate::application::swarm::dto::ReleaseTaskRequest;
use crate::application::swarm::use_cases::OverRepository;
use crate::domain::swarm::{BoardError, RefusalKind};

fn file(path: &str, task: i64, claim: &str) -> StoredFile {
    StoredFile {
        path: path.to_owned(),
        task,
        owner: "worker".to_owned(),
        claim: claim.to_owned(),
        token: "reservation".to_owned(),
    }
}

/// Task 1 claimed by the worker (token `stored-token`), task 2 by the
/// parent, task 3 ready; files reserved under task 1's claim, an older
/// claim of task 1, and task 2's claim.
fn board() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    state.tasks = vec![
        stored_task(1, "claimed", json!([]), Some("worker")),
        stored_task(2, "claimed", json!([]), Some("parent")),
        stored_task(3, "ready", json!([]), None),
    ];
    state.files = vec![
        file("a", 1, "stored-token"),
        file("b", 1, "older"),
        file("c", 2, "stored-token"),
    ];
    state
}

fn release(task_id: Value, token: Value) -> ReleaseTaskRequest {
    ReleaseTaskRequest {
        actor: "worker".to_owned(),
        task_id,
        token,
    }
}

/// A stale token, another member's claim and unclaimed work are refused;
/// the owner's release reopens the task, drops only the files reserved
/// under that claim and records `released{task}`.
#[test]
fn only_the_current_claim_releases_and_its_files_go() {
    let board = MemoryBoard::with(board());
    let service = ReleaseTask::new(board.clone(), SteppingClock::fixed(50.0));
    for (task_id, token) in [
        (json!(1), json!("stale")),
        (json!(1), json!(null)),
        (json!(2), json!("stored-token")),
        (json!(3), json!(null)),
    ] {
        assert_eq!(
            service
                .execute(release(task_id.clone(), token))
                .unwrap_err(),
            BoardError::new(RefusalKind::StaleToken, "stale or unowned claim"),
            "{task_id}"
        );
    }
    assert_eq!(
        service.execute(release(json!(9), json!("x"))).unwrap_err(),
        BoardError::new(RefusalKind::NotFound, "unknown task")
    );
    service
        .execute(release(json!(true), json!("stored-token")))
        .unwrap();
    let state = board.snapshot();
    let task = &state.tasks[0];
    assert_eq!(task.text("status"), Some("ready"));
    for column in ["owner", "token", "blocker"] {
        assert_eq!(task.get(column), Some(&Value::Null), "{column}");
    }
    let paths: Vec<&str> = state.files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["b", "c"]);
    assert_eq!(state.events.len(), 1);
    assert_eq!(
        (state.events[0].action.as_str(), &state.events[0].detail),
        ("released", &json!({"task": true}))
    );
}

/// Served over another repository (#2303 reconcile), the release is made
/// on that board alone, and answers the id the task's row holds, not the
/// one the caller gave (`true` binds to task 1).
#[test]
fn over_releases_on_the_given_board_and_answers_the_stored_id() {
    let composed_over = MemoryBoard::with(board());
    let other = MemoryBoard::with(board());
    let task = ReleaseTask::new(composed_over.clone(), SteppingClock::fixed(50.0))
        .over(other.clone())
        .execute(release(json!(true), json!("stored-token")))
        .unwrap();
    assert_eq!(task, json!(1));
    assert_eq!(other.snapshot().tasks[0].text("status"), Some("ready"));
    assert!(composed_over.transactions().is_empty());
}
