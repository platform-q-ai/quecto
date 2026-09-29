use serde_json::json;

use super::ReadTask;
use crate::application::swarm::board_test_support::{
    MemoryBoard, SteppingClock, member_row, running_board, stored_task,
};
use crate::application::swarm::dto::ReadTaskRequest;
use crate::application::swarm::use_cases::OverRepository;
use crate::domain::swarm::{BoardError, RefusalKind};

fn read(actor: &str, task_id: serde_json::Value) -> ReadTaskRequest {
    ReadTaskRequest {
        actor: actor.to_owned(),
        task_id,
    }
}

/// `_task`: a ready task whose dependencies are not all completed reads
/// `blocked` with the blocker `unmet dependencies`, in place (the row's
/// column order is kept); a completed dependency set leaves it ready, and
/// the stored row is never changed.
#[test]
fn a_ready_task_with_incomplete_dependencies_reads_blocked_with_unmet_dependencies() {
    let mut state = running_board(100.0);
    state.tasks = vec![
        stored_task(1, "completed", json!([]), None),
        stored_task(2, "claimed", json!([]), Some("parent")),
        stored_task(3, "ready", json!([1, 2]), None),
        stored_task(4, "ready", json!([1]), None),
        stored_task(5, "claimed", json!([2]), Some("parent")),
    ];
    let board = MemoryBoard::with(state);
    let service = ReadTask::new(board.clone(), SteppingClock::fixed(50.0));
    let blocked = service.execute(read("parent", json!(3))).unwrap();
    assert_eq!(blocked.text("status"), Some("blocked"));
    assert_eq!(blocked.text("blocker"), Some("unmet dependencies"));
    let columns: Vec<&str> = blocked
        .columns
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(
        columns,
        [
            "id",
            "title",
            "acceptance",
            "dependencies",
            "status",
            "owner",
            "token",
            "evidence",
            "blocker"
        ]
    );
    let ready = service.execute(read("parent", json!(4))).unwrap();
    assert_eq!(ready.text("status"), Some("ready"));
    assert_eq!(ready.get("blocker"), Some(&json!(null)));
    let claimed = service.execute(read("parent", json!(5))).unwrap();
    assert_eq!(
        claimed.text("status"),
        Some("claimed"),
        "only a ready task is derived"
    );
    assert_eq!(board.snapshot().tasks[2].text("status"), Some("ready"));
    assert_eq!(
        service.execute(read("parent", json!(6))).unwrap_err(),
        BoardError::new(RefusalKind::NotFound, "unknown task")
    );
}

/// A read-only operation: a dead member may read, a stranger may not.
#[test]
fn a_dead_member_reads_and_a_stranger_does_not() {
    let mut state = running_board(100.0);
    state.members.push(member_row("gone", "dead"));
    state.tasks = vec![stored_task(1, "ready", json!([]), None)];
    let service = ReadTask::new(MemoryBoard::with(state), SteppingClock::fixed(50.0));
    assert_eq!(
        service
            .execute(read("gone", json!(1)))
            .unwrap()
            .text("status"),
        Some("ready")
    );
    assert_eq!(
        service.execute(read("stranger", json!(1))).unwrap_err(),
        BoardError::new(
            RefusalKind::NotMember,
            "invoking member is unknown or death confirmed"
        )
    );
}

/// Served over another repository (#2303 reconcile), the task is read from
/// that board alone.
#[test]
fn over_reads_the_given_board() {
    let mut state = running_board(100.0);
    state.tasks = vec![stored_task(1, "ready", json!([]), None)];
    let composed_over = MemoryBoard::with(running_board(100.0));
    let other = MemoryBoard::with(state);
    let task = ReadTask::new(composed_over.clone(), SteppingClock::fixed(50.0))
        .over(other.clone())
        .execute(read("parent", json!(1)))
        .unwrap();
    assert_eq!(task.text("status"), Some("ready"));
    assert!(!other.transactions().is_empty());
    assert!(composed_over.transactions().is_empty());
}
