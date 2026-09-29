use serde_json::{Value, json};

use super::SubmitTask;
use crate::application::swarm::board_test_support::{
    BoardState, CompactEncoding, MemoryBoard, SteppingClock, member_row, running_board, stored_task,
};
use crate::application::swarm::dto::{SubmitTaskRequest, TaskTransition};
use crate::domain::swarm::{BoardError, RefusalKind};

/// Task 1 blocked by the worker under `stored-token`, task 2 claimed by
/// the parent.
fn board() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    let mut blocked = stored_task(1, "blocked", json!([]), Some("worker"));
    blocked.set("blocker", json!("awaiting approval"));
    state.tasks = vec![
        blocked,
        stored_task(2, "claimed", json!([]), Some("parent")),
    ];
    state
}

fn submit(task_id: Value, token: &str, evidence: Value) -> SubmitTaskRequest {
    SubmitTaskRequest {
        actor: "worker".to_owned(),
        task_id,
        token: Value::from(token),
        evidence,
    }
}

fn service(board: &std::sync::Arc<MemoryBoard>) -> SubmitTask {
    SubmitTask::new(
        board.clone(),
        SteppingClock::fixed(50.0),
        std::sync::Arc::new(CompactEncoding),
    )
}

/// Evidence is a nonempty list of dicts, each with a truthy `artifact` and
/// `revision`, its encoding bounded: every other shape is refused before
/// the gate.
#[test]
fn evidence_must_name_an_artifact_and_a_revision() {
    let board = MemoryBoard::with(board());
    let service = service(&board);
    for evidence in [
        json!(null),
        json!({"artifact": "a", "revision": "r"}),
        json!([]),
        json!(["a"]),
        json!([{"artifact": "a"}]),
        json!([{"artifact": "", "revision": "r"}]),
        json!([{"artifact": "a", "revision": 0}]),
        json!([{"artifact": [], "revision": "r"}]),
        json!([{"artifact": "a", "revision": "r"}, {"artifact": "b", "revision": false}]),
    ] {
        assert_eq!(
            service
                .execute(submit(json!(1), "stored-token", evidence.clone()))
                .unwrap_err(),
            BoardError::new(
                RefusalKind::Invalid,
                "artifact and revision evidence required"
            ),
            "{evidence}"
        );
    }
    // `[{"artifact":"<text>","revision":"r"}]` encodes to 32 bytes plus
    // the text.
    let long = json!([{"artifact": "a".repeat(8161), "revision": "r"}]);
    assert_eq!(
        service
            .execute(submit(json!(1), "stored-token", long))
            .unwrap_err(),
        BoardError::new(
            RefusalKind::Invalid,
            "evidence references must be nonempty and at most 8192 bytes"
        )
    );
    assert!(board.transactions().is_empty());
    let fits = json!([{"artifact": "a".repeat(8160), "revision": "r"}]);
    service
        .execute(submit(json!(1), "stored-token", fits))
        .unwrap();
}

/// `test_reviewed_submission_cannot_be_replaced_under_the_same_claim`:
/// the owner's submission stores the evidence as given, clears the
/// blocker and records `submitted{task}`; the same evidence again (by
/// Python's `==`) is a no-op, and other evidence is refused as immutable.
#[test]
fn a_submission_is_immutable_under_its_claim() {
    let board = MemoryBoard::with(board());
    let service = service(&board);
    let evidence = json!([{"artifact": "reviewed-A", "revision": "R1", "n": 1}]);
    assert_eq!(
        service
            .execute(submit(json!(1), "stale", evidence.clone()))
            .unwrap_err(),
        BoardError::new(RefusalKind::StaleToken, "stale or unowned claim")
    );
    assert_eq!(
        service
            .execute(submit(json!(2), "stored-token", evidence.clone()))
            .unwrap_err(),
        BoardError::new(RefusalKind::StaleToken, "stale or unowned claim")
    );
    assert_eq!(
        service
            .execute(submit(json!("1"), "stored-token", evidence.clone()))
            .unwrap(),
        TaskTransition::Applied
    );
    let same = json!([{"n": 1.0, "revision": "R1", "artifact": "reviewed-A"}]);
    assert_eq!(
        service
            .execute(submit(json!(1), "stored-token", same))
            .unwrap(),
        TaskTransition::Unchanged
    );
    assert_eq!(
        service
            .execute(submit(
                json!(1),
                "stored-token",
                json!([{"artifact": "unreviewed-B", "revision": "R1"}])
            ))
            .unwrap_err(),
        BoardError::new(
            RefusalKind::Immutable,
            "submitted evidence is immutable; release and reclaim before revising"
        )
    );
    let state = board.snapshot();
    let task = &state.tasks[0];
    assert_eq!(task.text("status"), Some("submitted"));
    assert_eq!(task.get("evidence"), Some(&evidence));
    assert_eq!(task.get("blocker"), Some(&Value::Null));
    assert_eq!(state.events.len(), 1);
    assert_eq!(
        (state.events[0].action.as_str(), &state.events[0].detail),
        ("submitted", &json!({"task": "1"}))
    );
}
