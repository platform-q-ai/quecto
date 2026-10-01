use serde_json::{Value, json};

use super::SetTaskDependencies;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, SteppingClock, member_row, running_board, stored_task,
};
use crate::application::swarm::dto::SetTaskDependenciesRequest;
use crate::application::swarm::use_cases::OverRepository;
use crate::domain::swarm::{BoardError, RefusalKind};

/// Tasks 1 (completed), 2 (ready, on 1), 3 (claimed by the worker).
fn board() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    state.tasks = vec![
        stored_task(1, "completed", json!([]), None),
        stored_task(2, "ready", json!([1]), None),
        stored_task(3, "claimed", json!([]), Some("worker")),
    ];
    state
}

/// Each refusal's kind (#2303).
fn kind(refusal: &str) -> RefusalKind {
    match refusal {
        "unknown task" => RefusalKind::NotFound,
        "dependencies may change only before claiming" => RefusalKind::WrongState,
        "invalid, missing or self dependencies" | "dependencies must be a bounded list" => {
            RefusalKind::Invalid
        }
        other => panic!("no kind listed for {other:?}"),
    }
}

fn set(task_id: Value, dependencies: Value) -> SetTaskDependenciesRequest {
    SetTaskDependenciesRequest {
        actor: "worker".to_owned(),
        task_id,
        dependencies,
    }
}

/// `test_dependencies_reject_missing_self_and_cycles`: a missing id, a
/// self edge and a cycle are refused and nothing changes; a valid change
/// is stored and recorded as `dependencies{task,dependencies}`, the task id
/// as the caller gave it.
#[test]
fn dependencies_reject_missing_self_and_cycles() {
    let board = MemoryBoard::with(board());
    let service = SetTaskDependencies::new(board.clone(), SteppingClock::fixed(50.0));
    for (task_id, dependencies, refusal) in [
        (
            json!(1),
            json!([99_999]),
            "dependencies may change only before claiming",
        ),
        (
            json!(2),
            json!([99_999]),
            "invalid, missing or self dependencies",
        ),
        (
            json!(2),
            json!([2]),
            "invalid, missing or self dependencies",
        ),
        (json!(2), json!(null), "dependencies must be a bounded list"),
        (
            json!(3),
            json!([1]),
            "dependencies may change only before claiming",
        ),
        (json!(9), json!([1]), "unknown task"),
    ] {
        assert_eq!(
            service
                .execute(set(task_id.clone(), dependencies.clone()))
                .unwrap_err(),
            BoardError::new(kind(refusal), refusal),
            "{task_id} -> {dependencies}"
        );
    }
    assert!(board.snapshot().events.is_empty());
    service.execute(set(json!("2"), json!([]))).unwrap();
    let state = board.snapshot();
    assert_eq!(state.tasks[1].get("dependencies"), Some(&json!([])));
    assert_eq!(state.events.len(), 1);
    assert_eq!(
        (state.events[0].action.as_str(), &state.events[0].detail),
        ("dependencies", &json!({"task": "2", "dependencies": []}))
    );
}

/// A ready task blocked by unmet dependencies may still change them; a
/// cycle through the new edge is refused.
#[test]
fn a_blocked_unclaimed_task_may_change_and_cycles_are_refused() {
    let mut state = board();
    state.tasks.push(stored_task(4, "ready", json!([2]), None));
    state.tasks[0].set("status", json!("ready"));
    let board = MemoryBoard::with(state);
    let service = SetTaskDependencies::new(board.clone(), SteppingClock::fixed(50.0));
    assert_eq!(
        service.execute(set(json!(1), json!([4]))).unwrap_err(),
        BoardError::new(RefusalKind::DependencyCycle, "cyclic dependencies")
    );
    service.execute(set(json!(4), json!([1]))).unwrap();
    assert_eq!(
        board.snapshot().tasks[3].get("dependencies"),
        Some(&json!([1]))
    );
}

/// Served over another repository (#2303 reconcile), the change is made on
/// that board alone, and answers the id the task's row holds, not the one
/// the caller gave (`"2"` binds to task 2).
#[test]
fn over_changes_the_given_board_and_answers_the_stored_id() {
    let composed_over = MemoryBoard::with(board());
    let other = MemoryBoard::with(board());
    let task = SetTaskDependencies::new(composed_over.clone(), SteppingClock::fixed(50.0))
        .over(other.clone())
        .execute(set(json!("2"), json!([])))
        .unwrap();
    assert_eq!(
        (task.get("id"), task.get("dependencies")),
        (Some(&json!(2)), Some(&json!([])))
    );
    assert_eq!(other.snapshot().events.len(), 1);
    assert!(composed_over.transactions().is_empty());
}
