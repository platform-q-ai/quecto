//! #2279 review M1: what the post-call lifecycle leaves on a structured
//! op's answer. An answer that is not an object rides beside the notes as
//! `{"result": …}`, and a lifecycle that fails rides the answer as
//! `coordination_error`. And (review M2) a store the gate cannot read
//! refuses the op as the op's own store refusal.
use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::{Value, json};

use super::{
    CountingLifecycle, Recorded, answered, board, board_with, deadline, direct, execute,
    rejecting_worker,
};
use crate::domain::swarm::{BoardOpOutcome, RefusalKind};
use crate::infrastructure::tools::swarm_bridge::SwarmContext;

/// `dependencies` answers `None`; setting them on a ready task hands it to
/// the free worker, whose endpoint refuses the wake hint. The answer is
/// `{"result": null, "notification_warnings": […]}`, written as Python
/// writes it, the result first.
#[tokio::test]
async fn a_null_answer_rides_beside_the_wake_warnings_as_its_result() {
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
        result.content.starts_with(
            r#"{"result": null, "notification_warnings": ["wake hint failed for worker"#
        ),
        "{}",
        result.content
    );
    let answer: Value = serde_json::from_str(&result.content).unwrap();
    let fields = answer.as_object().unwrap();
    assert_eq!(fields.len(), 2, "{answer}");
    assert_eq!(fields["result"], Value::Null);
    assert_eq!(fields["notification_warnings"].as_array().unwrap().len(), 1);
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
