use serde_json::{Value, json};

use super::CreateTask;
use crate::application::swarm::board_test_support::{
    BoardState, CompactEncoding, MemoryBoard, SteppingClock, member_row, running_board, stored_task,
};
use crate::application::swarm::dto::CreateTaskRequest;
use crate::application::swarm::use_cases::OverRepository;
use crate::domain::swarm::{BoardError, RefusalKind};

const ACCEPTANCE: &str =
    "task acceptance criteria required: use a nonempty list[str], e.g. ['tests pass']";

fn board() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    state
}

fn service(board: &std::sync::Arc<MemoryBoard>) -> CreateTask {
    CreateTask::new(
        board.clone(),
        SteppingClock::fixed(50.0),
        std::sync::Arc::new(CompactEncoding),
    )
}

fn create(request: &str, title: &str, dependencies: Value) -> CreateTaskRequest {
    CreateTaskRequest {
        actor: "worker".to_owned(),
        request: json!(request),
        title: json!(title),
        acceptance: json!(["tests pass"]),
        dependencies,
    }
}

/// A new request creates the task, records `task_created{task}` and stores
/// the task's dict as the request's result; the same request replays it
/// without a second task or event, and the same id with another payload is
/// refused.
#[test]
fn request_retries_are_idempotent_but_payload_conflicts_fail() {
    let board = MemoryBoard::with(board());
    let service = service(&board);
    let created = service.execute(create("r1", "work", json!(null))).unwrap();
    assert!(!created.replayed);
    assert_eq!(created.task["id"], json!(1));
    assert_eq!(created.task["status"], json!("ready"));
    assert_eq!(created.task["acceptance"], json!(["tests pass"]));
    let again = service.execute(create("r1", "work", json!([]))).unwrap();
    assert!(again.replayed, "None and [] are the same payload");
    assert_eq!(again.task, created.task);
    assert_eq!(
        service
            .execute(create("r1", "different", json!([])))
            .unwrap_err(),
        BoardError::new(
            RefusalKind::RequestIdReused,
            "request id reused with different payload"
        )
    );
    let state = board.snapshot();
    assert_eq!(state.tasks.len(), 1);
    assert_eq!(state.events.len(), 1);
    assert_eq!(
        (state.events[0].action.as_str(), &state.events[0].detail),
        ("task_created", &json!({"task": 1}))
    );
    assert_eq!(state.requests.len(), 1);
    assert_eq!(
        state.requests[0].payload,
        json!(["task", "work", ["tests pass"], []])
    );
    assert_eq!(state.requests[0].result, created.task);
}

/// `dependencies or []`: every falsy value is no dependencies.
#[test]
fn falsy_dependencies_are_none() {
    for (index, falsy) in [json!(null), json!(0), json!(false), json!(""), json!({})]
        .into_iter()
        .enumerate()
    {
        let board = MemoryBoard::with(board());
        let created = service(&board)
            .execute(create(&format!("r{index}"), "work", falsy.clone()))
            .unwrap();
        assert_eq!(created.task["dependencies"], json!([]), "{falsy}");
        assert_eq!(
            board.snapshot().requests[0].payload[3],
            json!([]),
            "{falsy}"
        );
    }
}

/// The title and acceptance are checked before the operation (a stranger
/// is refused them first); the request id inside it, after authorisation.
#[test]
fn arguments_are_checked_in_pythons_order() {
    let board = MemoryBoard::with(board());
    let service = service(&board);
    let mut stranger = create("r1", "", json!(null));
    stranger.actor = "stranger".to_owned();
    assert_eq!(
        service.execute(stranger.clone()).unwrap_err(),
        BoardError::new(
            RefusalKind::Invalid,
            "title must be nonempty and at most 1024 bytes"
        )
    );
    let long = "a".repeat(1025);
    assert_eq!(
        service
            .execute(create("r1", &long, json!(null)))
            .unwrap_err(),
        BoardError::new(
            RefusalKind::Invalid,
            "title must be nonempty and at most 1024 bytes"
        )
    );
    for invalid in [
        json!("tests pass"),
        json!([]),
        json!([42]),
        json!([""]),
        json!([" \t"]),
    ] {
        let mut request = create("r1", "work", json!(null));
        request.acceptance = invalid.clone();
        assert_eq!(
            service.execute(request).unwrap_err(),
            BoardError::new(RefusalKind::Invalid, ACCEPTANCE),
            "{invalid}"
        );
    }
    let mut request = create("r1", "work", json!(null));
    request.acceptance = json!(["é".repeat(1366)]);
    assert_eq!(
        service.execute(request).unwrap_err(),
        BoardError::new(
            RefusalKind::Invalid,
            "acceptance must be nonempty and at most 8192 bytes"
        ),
        "the encoded list is bounded: é is six bytes escaped"
    );
    stranger.title = json!("work");
    assert_eq!(
        service.execute(stranger).unwrap_err(),
        BoardError::new(
            RefusalKind::NotMember,
            "invoking member is unknown or death confirmed"
        )
    );
    for request_id in [json!(5), json!(""), json!("r".repeat(129))] {
        let mut request = create("r1", "work", json!(null));
        request.request = request_id.clone();
        assert_eq!(
            service.execute(request).unwrap_err(),
            BoardError::new(
                RefusalKind::Invalid,
                "request id must be nonempty and at most 128 bytes"
            ),
            "{request_id}"
        );
    }
    assert!(board.snapshot().tasks.is_empty());
}

/// The cap is checked before the insert; invalid dependencies are checked
/// after it, against the new id, and roll it back.
#[test]
fn a_full_board_and_invalid_dependencies_create_nothing() {
    let mut state = board();
    state.tasks = (1..=1000)
        .map(|id| stored_task(id, "ready", json!([]), None))
        .collect();
    let full = MemoryBoard::with(state);
    assert_eq!(
        service(&full)
            .execute(create("r1", "work", json!(null)))
            .unwrap_err(),
        BoardError::new(
            RefusalKind::CapacityFull,
            "task board full (1000); settle existing work"
        )
    );
    assert_eq!(full.journal(), Vec::<String>::new(), "nothing inserted");

    let board = MemoryBoard::with(board());
    let service = service(&board);
    for (dependencies, refusal) in [
        (json!([1]), "invalid, missing or self dependencies"),
        (json!([7]), "invalid, missing or self dependencies"),
        (json!("abc"), "dependencies must be a bounded list"),
    ] {
        assert_eq!(
            service
                .execute(create("r1", "work", dependencies.clone()))
                .unwrap_err(),
            BoardError::new(RefusalKind::Invalid, refusal),
            "{dependencies}"
        );
    }
    let state = board.snapshot();
    assert!(state.tasks.is_empty() && state.events.is_empty() && state.requests.is_empty());
    assert_eq!(
        board.journal()[..2],
        [
            "insert_task 1".to_owned(),
            "all_task_dependencies".to_owned()
        ],
        "the insert comes before the dependency check"
    );
}

/// A task created with an incomplete dependency is answered blocked.
#[test]
fn a_dependent_task_is_created_blocked() {
    let board = MemoryBoard::with(board());
    let service = service(&board);
    service.execute(create("r1", "first", json!(null))).unwrap();
    let dependent = service.execute(create("r2", "second", json!([1]))).unwrap();
    assert_eq!(dependent.task["status"], json!("blocked"));
    assert_eq!(dependent.task["blocker"], json!("unmet dependencies"));
    assert_eq!(dependent.task["dependencies"], json!([1]));
    assert_eq!(board.snapshot().tasks[1].text("status"), Some("ready"));
}

/// Served over another repository (#2303 reconcile), the task is created
/// on that board alone, encoded as composed.
#[test]
fn over_creates_on_the_given_board() {
    let composed_over = MemoryBoard::with(board());
    let other = MemoryBoard::with(board());
    let created = service(&composed_over)
        .over(other.clone())
        .execute(create("r1", "work", json!(null)))
        .unwrap();
    assert_eq!(created.task["id"], json!(1));
    assert_eq!(other.snapshot().tasks.len(), 1);
    assert!(composed_over.transactions().is_empty());
    assert!(composed_over.snapshot().tasks.is_empty());
}
