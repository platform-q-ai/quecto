//! #2279 review M1: what the post-call lifecycle leaves on a structured
//! op's answer. An answer that is not an object rides beside the notes as
//! `{"result": …}`, and a lifecycle that fails rides the answer as
//! `coordination_error`; a refusal keeps both (review L4). And (review M2) a store the gate cannot read
//! refuses the op as the op's own store refusal.
use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::{Value, json};

use super::{
    CountingLifecycle, Recorded, answered, board, board_with, deadline, direct, execute,
    rejecting_worker,
};
use crate::domain::swarm::{BoardOpOutcome, BoardRole, RefusalKind};
use crate::infrastructure::tools::swarm_bridge::SwarmContext;

/// `dependencies` answers the task's row (#2394); setting them on a ready
/// task hands it to the free worker, whose endpoint refuses the wake hint.
/// The warnings are added to the row, after its own fields.
#[tokio::test]
async fn an_object_answer_carries_the_wake_warnings() {
    let (_directory, context) = board();
    direct(&context, "task_create", json!(["r1", "t", ["pass"]])).unwrap();
    let recipient = rejecting_worker(&context);
    let result = execute(
        &context,
        r#"{"op": "dependencies", "task_id": 1, "dependencies": []}"#,
    )
    .await;
    recipient.join().unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert!(
        result.content.starts_with(r#"{"id": 1, "#),
        "{}",
        result.content
    );
    let answer: Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(
        (&answer["status"], &answer["dependencies"]),
        (&json!("ready"), &json!([])),
        "{answer}"
    );
    let warnings = answer["notification_warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "{answer}");
    assert!(
        warnings[0]
            .as_str()
            .unwrap()
            .starts_with("wake hint failed for worker"),
        "{answer}"
    );
}

/// An answer that is not an object (no member-facing op answers one since
/// #2394, so this is the defensive path) rides beside the notes as
/// `{"result": …}`, written as Python writes it, the result first.
#[test]
fn a_non_object_answer_rides_beside_the_notes_as_its_result() {
    let mut notes = serde_json::Map::new();
    notes.insert("notification_warnings".to_owned(), json!(["w"]));
    let result = crate::infrastructure::tools::swarm_board_ops::answered(
        &crate::composition::swarm::board_wire(),
        Ok(Value::Null),
        notes,
    )
    .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        result.content,
        r#"{"result": null, "notification_warnings": ["w"]}"#
    );
}

/// `stop` ends the run, so the lifecycle settles it; the settlement fails,
/// and its error rides the board's receipt as `coordination_error`, the
/// stop itself answered.
#[tokio::test]
async fn a_failed_lifecycle_rides_the_answer_as_coordination_error() {
    let lifecycle = Arc::new(CountingLifecycle::default());
    lifecycle.fail_settle.store(true, Ordering::SeqCst);
    let (_directory, context) = board_with(deadline(), lifecycle.clone());
    let receipt = answered(
        &context,
        json!({"op": "stop", "status": "blocked", "reason": "needs a decision"}),
    )
    .await;
    assert_eq!(receipt["outcome"], "blocked", "{receipt}");
    assert_eq!(lifecycle.settled.load(Ordering::SeqCst), 1, "{receipt}");
    assert_eq!(
        receipt["coordination_error"], "tool error: settlement unavailable",
        "{receipt}"
    );
    assert!(receipt.get("notification_warnings").is_none(), "{receipt}");
}

/// A checkout with no board store: the gate's `_status` read fails, and
/// the op is refused with the store's text and recorded as its own
/// refusal of kind `store`, beside the failed `_status` read's record.
#[tokio::test]
async fn a_store_the_gate_cannot_read_refuses_the_op_as_its_own() {
    let directory = tempfile::tempdir().unwrap();
    let context = SwarmContext {
        board: crate::composition::swarm::swarm_board(),
        checkout: directory.path().to_path_buf(),
        member: "worker".into(),
        lifecycle: Arc::new(crate::application::swarm::LifecycleService),
    };
    let log = Arc::new(Recorded::default());
    assert!(context.board.record_in(log.clone()));
    let result = execute(&context, r#"{"op": "claim", "task_id": 1}"#).await;
    assert!(result.is_error, "{}", result.content);
    assert!(
        result.content.starts_with("tool error: swarm: ")
            && result.content.contains("coordination store missing"),
        "{}",
        result.content
    );
    let records = log.take();
    let outcome = |op: &str| {
        records
            .iter()
            .filter(|record| record.op == op)
            .map(|record| record.outcome)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        outcome("claim"),
        [BoardOpOutcome::Refused {
            kind: RefusalKind::Store,
            committed: false,
        }],
        "{records:?}"
    );
    assert_eq!(outcome("_status").len(), 1, "{records:?}");
}

/// Review L4: a refused op whose call still moved the cursor (another
/// member wrote meanwhile) keeps the lifecycle's notes, as `op=run` keeps
/// them whatever the program's outcome: the refusal text as the board
/// refused, then the notes as Python writes them.
#[test]
fn a_refusal_keeps_the_lifecycle_notes() {
    let mut notes = serde_json::Map::new();
    notes.insert(
        "notification_warnings".to_owned(),
        json!(["wake hint failed for worker; durable board is authoritative"]),
    );
    notes.insert("coordination_error".to_owned(), json!("tool error: gone"));
    let refusal = crate::domain::error::DomainError::Tool(r#"swarm: "unknown task""#.to_owned());
    let result = super::super::answered(
        &crate::composition::swarm::board_wire(),
        Err(refusal),
        notes,
    )
    .unwrap();
    assert!(result.is_error, "{}", result.content);
    assert_eq!(
        result.content,
        concat!(
            r#"tool error: swarm: "unknown task""#,
            "\n",
            r#"{"notification_warnings": ["wake hint failed for worker; durable board is authoritative"], "coordination_error": "tool error: gone"}"#
        )
    );
}

/// Without notes a refusal is exactly the board's.
#[test]
fn a_refusal_without_notes_is_the_boards() {
    let refusal = crate::domain::error::DomainError::Tool(r#"swarm: "unknown task""#.to_owned());
    let result = super::super::answered(
        &crate::composition::swarm::board_wire(),
        Err(refusal),
        serde_json::Map::new(),
    )
    .unwrap();
    assert!(result.is_error);
    assert_eq!(result.content, r#"tool error: swarm: "unknown task""#);
}

/// Each record's op and role, taken from `log`.
fn roles(log: &Recorded) -> Vec<(String, Option<BoardRole>)> {
    log.take()
        .into_iter()
        .map(|record| (record.op, record.role))
        .collect()
}

/// #2279 S15 final review: the board reads the post-call lifecycle makes
/// (the summary it judges the run by, the settlement's snapshot and
/// reconcile) are the harness's, recorded as `host`, never counted against
/// the member whose op moved the cursor; the member's own op, and a
/// `summary` the member asks for, keep the member's role.
#[tokio::test]
async fn the_lifecycle_s_board_reads_are_recorded_as_host() {
    use BoardRole::{Coordinator, Host};
    let (_directory, context) = board();
    direct(&context, "task_create", json!(["r1", "t", ["pass"]])).unwrap();
    let log = Arc::new(Recorded::default());
    assert!(context.board.record_in(log.clone()));
    answered(&context, json!({"op": "claim", "task_id": 1})).await;
    let host = |op: &str| (op.to_owned(), Some(Host));
    assert_eq!(
        roles(&log),
        [
            host("_status"),
            host("_event_cursor"),
            ("claim".to_owned(), Some(Coordinator)),
            host("_event_cursor"),
            host("summary"),
            host("_notifications"),
        ]
    );
    answered(&context, json!({"op": "summary"})).await;
    assert_eq!(roles(&log), [("summary".to_owned(), Some(Coordinator))]);
    answered(
        &context,
        json!({"op": "stop", "status": "blocked", "reason": "needs a decision"}),
    )
    .await;
    let stopped = roles(&log);
    assert_eq!(
        stopped[..4],
        [
            host("_status"),
            host("_event_cursor"),
            ("stop".to_owned(), Some(Coordinator)),
            host("_event_cursor"),
        ],
        "{stopped:?}"
    );
    assert!(
        stopped[4..].iter().all(|(_, role)| *role == Some(Host)),
        "the lifecycle and the settlement read as host: {stopped:?}"
    );
    assert!(
        stopped[4..].iter().any(|(op, _)| op == "summary"),
        "{stopped:?}"
    );
}
