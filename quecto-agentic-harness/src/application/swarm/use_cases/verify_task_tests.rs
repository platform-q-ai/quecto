use serde_json::{Value, json};

use super::VerifyTask;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, SteppingClock, StoredFile, member_row, running_board, stored_task,
};
use crate::application::swarm::dto::{TaskTransition, VerifyTaskRequest};
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

/// Task 1 submitted by the worker under `stored-token` at revision `R1`,
/// task 2 claimed by the worker; files reserved under task 1's claim, an
/// older claim of task 1, and task 2's claim.
fn board() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    let mut submitted = stored_task(1, "submitted", json!([]), Some("worker"));
    submitted.set(
        "evidence",
        json!([{"artifact": "a", "revision": "R1"}, {"artifact": "b", "revision": "R1"}]),
    );
    state.tasks = vec![
        submitted,
        stored_task(2, "claimed", json!([]), Some("worker")),
    ];
    state.files = vec![
        file("a", 1, "stored-token"),
        file("b", 1, "older"),
        file("c", 2, "stored-token"),
    ];
    state
}

fn verify(actor: &str, task_id: Value, token: &str, revision: Value) -> VerifyTaskRequest {
    VerifyTaskRequest {
        actor: actor.to_owned(),
        task_id,
        token: Value::from(token),
        revision,
    }
}

/// `test_submission_is_not_completion_and_requires_current_token`: only
/// the coordinator verifies; the submission's token and every entry's
/// revision must match. Verification completes the task, deletes the
/// files reserved under that claim only and records
/// `verified{task,revision}`; verifying completed work again is a no-op,
/// but still checks the revision.
#[test]
fn the_coordinator_verifies_the_current_submission_once() {
    let board = MemoryBoard::with(board());
    let service = VerifyTask::new(board.clone(), SteppingClock::fixed(50.0));
    for (request, refusal) in [
        (
            verify("worker", json!(1), "stored-token", json!("R1")),
            (
                RefusalKind::NotCoordinator,
                "only the designated coordinator may do this",
            ),
        ),
        (
            verify("parent", json!(1), "stale", json!("R1")),
            (RefusalKind::StaleToken, "stale claim or work not submitted"),
        ),
        (
            verify("parent", json!(2), "stored-token", json!("R1")),
            (RefusalKind::StaleToken, "stale claim or work not submitted"),
        ),
        (
            verify("parent", json!(1), "stored-token", json!("R2")),
            (RefusalKind::StaleRevision, "stale evidence revision"),
        ),
        (
            verify("parent", json!(9), "stored-token", json!("R1")),
            (RefusalKind::NotFound, "unknown task"),
        ),
    ] {
        assert_eq!(
            service.execute(request).unwrap_err(),
            BoardError::new(refusal.0, refusal.1)
        );
    }
    assert_eq!(
        service
            .execute(verify("parent", json!("1"), "stored-token", json!("R1")))
            .unwrap()
            .transition,
        TaskTransition::Applied
    );
    assert_eq!(
        service
            .execute(verify("parent", json!(1), "stored-token", json!("R1")))
            .unwrap()
            .transition,
        TaskTransition::Unchanged
    );
    assert_eq!(
        service
            .execute(verify("parent", json!(1), "stored-token", json!("R2")))
            .unwrap_err(),
        BoardError::new(RefusalKind::StaleRevision, "stale evidence revision")
    );
    let state = board.snapshot();
    assert_eq!(state.tasks[0].text("status"), Some("completed"));
    assert_eq!(state.tasks[0].text("owner"), Some("worker"));
    let paths: Vec<&str> = state.files.iter().map(|file| file.path.as_str()).collect();
    assert_eq!(paths, ["b", "c"]);
    assert_eq!(state.events.len(), 1);
    assert_eq!(
        (state.events[0].action.as_str(), &state.events[0].detail),
        ("verified", &json!({"task": "1", "revision": "R1"}))
    );
}

/// Revisions compare by Python's `==`: a stored integer revision matches
/// its float, and text never matches a number.
#[test]
fn revisions_compare_as_python_compares_them() {
    let mut state = board();
    state.tasks[0].set("evidence", json!([{"artifact": "a", "revision": 7}]));
    let board = MemoryBoard::with(state);
    let service = VerifyTask::new(board.clone(), SteppingClock::fixed(50.0));
    assert_eq!(
        service
            .execute(verify("parent", json!(1), "stored-token", json!("7")))
            .unwrap_err(),
        BoardError::new(RefusalKind::StaleRevision, "stale evidence revision")
    );
    service
        .execute(verify("parent", json!(1), "stored-token", json!(7.0)))
        .unwrap();
    assert_eq!(
        board.snapshot().events[0].detail,
        json!({"task": 1, "revision": 7.0})
    );
}

/// `outside_edited_evidence`: stored evidence only a file edited outside
/// the board can hold (not a list, or an entry without a `revision`) is
/// refused by name, where Python raises or iterates the value.
#[test]
fn stored_evidence_without_revisions_is_refused() {
    for evidence in [
        json!({"revision": "R1"}),
        json!("R1"),
        json!(null),
        json!([{"artifact": "a"}]),
        json!(["R1"]),
    ] {
        let mut state = board();
        state.tasks[0].set("evidence", evidence.clone());
        let board = MemoryBoard::with(state);
        let service = VerifyTask::new(board.clone(), SteppingClock::fixed(50.0));
        assert_eq!(
            service
                .execute(verify("parent", json!(1), "stored-token", json!("R1")))
                .unwrap_err(),
            BoardError::new(
                RefusalKind::Store,
                "stored evidence is not a list of revisioned entries"
            ),
            "{evidence}"
        );
        assert_eq!(board.snapshot().tasks[0].text("status"), Some("submitted"));
    }
}

/// Entries are read in order until the first stale one, as Python's `any`
/// reads them: a stale revision before an entry without one is refused as
/// stale.
#[test]
fn entries_are_read_until_the_first_stale_revision() {
    for (evidence, refusal) in [
        (
            json!([{"revision": "R2"}, "junk"]),
            (RefusalKind::StaleRevision, "stale evidence revision"),
        ),
        (
            json!([{"revision": "R1"}, "junk"]),
            (
                RefusalKind::Store,
                "stored evidence is not a list of revisioned entries",
            ),
        ),
    ] {
        let mut state = board();
        state.tasks[0].set("evidence", evidence.clone());
        let board = MemoryBoard::with(state);
        let service = VerifyTask::new(board, SteppingClock::fixed(50.0));
        assert_eq!(
            service
                .execute(verify("parent", json!(1), "stored-token", json!("R1")))
                .unwrap_err(),
            BoardError::new(refusal.0, refusal.1),
            "{evidence}"
        );
    }
}
