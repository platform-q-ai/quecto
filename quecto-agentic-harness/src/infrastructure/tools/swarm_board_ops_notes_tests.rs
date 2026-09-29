//! #2279 review M1: what the post-call lifecycle leaves on a structured
//! op's answer. An answer that is not an object rides beside the notes as
//! `{"result": …}`, and a lifecycle that fails rides the answer as
//! `coordination_error`.
use std::sync::Arc;
use std::sync::atomic::Ordering;

use serde_json::{Value, json};

use super::{
    CountingLifecycle, answered, board, board_with, deadline, direct, execute, rejecting_worker,
};

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
