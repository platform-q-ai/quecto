use serde_json::{Value, json};

use super::{owned, read_task, unsubmitted};
use crate::application::swarm::board_operation::atomic;
use crate::application::swarm::board_test_support::{MemoryBoard, running_board, stored_task};
use crate::domain::swarm::{BoardError, RefusalKind};

fn board(tasks: Vec<crate::application::swarm::dto::TaskRow>) -> std::sync::Arc<MemoryBoard> {
    let mut state = running_board(100.0);
    state.tasks = tasks;
    MemoryBoard::with(state)
}

/// Only a `ready` task is derived, and only while a dependency is not
/// `completed`: a missing dependency row counts as incomplete.
#[test]
fn only_a_ready_task_with_an_incomplete_dependency_reads_blocked() {
    let board = board(vec![
        stored_task(1, "completed", json!([]), None),
        stored_task(2, "ready", json!([1]), None),
        stored_task(3, "ready", json!([1, 9]), None),
        stored_task(4, "submitted", json!([9]), Some("worker")),
    ]);
    atomic(&*board, false, |transaction| {
        let status = |id: i64| {
            read_task(transaction, &json!(id)).map(|task| {
                (
                    task.text("status").map(str::to_owned),
                    task.get("blocker").cloned(),
                )
            })
        };
        assert_eq!(status(2)?, (Some("ready".to_owned()), Some(Value::Null)));
        assert_eq!(
            status(3)?,
            (
                Some("blocked".to_owned()),
                Some(json!("unmet dependencies"))
            )
        );
        assert_eq!(
            status(4)?,
            (Some("submitted".to_owned()), Some(Value::Null))
        );
        assert_eq!(
            status(5),
            Err(BoardError::new(RefusalKind::NotFound, "unknown task"))
        );
        Ok(())
    })
    .unwrap();
}

/// `_owned`: the token and the owner compare by Python's `==`, and the
/// task must hold a claim.
#[test]
fn owned_requires_the_token_the_owner_and_a_held_claim() {
    let mut blocked = stored_task(2, "blocked", json!([]), Some("worker"));
    blocked.set("token", json!(7));
    let board = board(vec![
        stored_task(1, "claimed", json!([]), Some("worker")),
        blocked,
        stored_task(3, "completed", json!([]), Some("worker")),
        stored_task(4, "ready", json!([]), None),
    ]);
    atomic(&*board, false, |transaction| {
        let stale = Err(BoardError::new(
            RefusalKind::StaleToken,
            "stale or unowned claim",
        ));
        let check = |id: i64, token: Value, member: &str| {
            owned(transaction, &json!(id), &token, member).map(|task| task.get("id").cloned())
        };
        assert_eq!(
            check(1, json!("stored-token"), "worker"),
            Ok(Some(json!(1)))
        );
        assert_eq!(check(1, json!("stored-token"), "other"), stale);
        assert_eq!(check(1, json!("stale"), "worker"), stale);
        assert_eq!(check(2, json!(7.0), "worker"), Ok(Some(json!(2))));
        assert_eq!(check(2, json!("7"), "worker"), stale);
        assert_eq!(check(3, json!("stored-token"), "worker"), stale);
        assert_eq!(check(4, json!(null), "worker"), stale);
        Ok(())
    })
    .unwrap();
}

#[test]
fn unsubmitted_uses_only_status_and_refuses_submitted_evidence() {
    let mut claimed = stored_task(1, "claimed", json!([]), Some("worker"));
    claimed.set("id", Value::Null);
    claimed.set("evidence", json!({"extra": "not a list"}));
    assert_eq!(unsubmitted(&claimed), Ok(()));

    let submitted = stored_task(2, "submitted", json!([]), Some("worker"));
    assert_eq!(
        unsubmitted(&submitted),
        Err(BoardError::new(
            RefusalKind::Immutable,
            "submitted evidence is immutable; release and reclaim before revising"
        ))
    );
}

#[test]
#[should_panic(expected = "only a held claim is checked")]
fn unsubmitted_asserts_a_held_status_even_in_release() {
    let ready = stored_task(1, "ready", json!([]), None);
    let _ = unsubmitted(&ready);
}

#[test]
#[should_panic(expected = "only a held claim is checked")]
fn unsubmitted_asserts_when_status_is_missing() {
    let mut task = stored_task(1, "claimed", json!([]), Some("worker"));
    task.set("status", Value::Null);
    let _ = unsubmitted(&task);
}
