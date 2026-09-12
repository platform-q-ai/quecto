//! Contract for [`TurnCancellation`] (#1934): cancellation is the first
//! effect of an executed shutdown, runs exactly once across joined callers,
//! and reports truthfully whether a turn was interrupted.
use std::sync::Arc;
use std::sync::atomic::Ordering;

use quecto::application::subagents::ports::TurnCancellation;
use quecto::domain::subagent_teardown::ShutdownReason;

use super::teardown_fixture::{Cancellation, Harness, root_tree};

#[tokio::test]
async fn port_is_object_safe_and_reports_whether_a_turn_was_interrupted() {
    let port: Arc<dyn TurnCancellation> = Cancellation::with_turn();
    assert!(port.cancel_in_flight_turn().await);
    assert!(!port.cancel_in_flight_turn().await);
}

#[tokio::test]
async fn executed_shutdown_cancels_once_before_any_child_is_addressed() {
    let harness = Harness::new(root_tree());
    let prepared = harness.prepared(ShutdownReason::TerminationSignal);
    assert_eq!(harness.cancellation.calls.load(Ordering::SeqCst), 0);
    let outcome = harness.execute.execute(&prepared.token).await.unwrap();
    assert!(outcome.turn_cancelled);
    assert_eq!(harness.cancellation.calls.load(Ordering::SeqCst), 1);
    // Idle harness: nothing to interrupt, reported as such.
    let idle = Harness::new(root_tree());
    idle.cancellation.in_flight.store(false, Ordering::SeqCst);
    let prepared = idle.prepared(ShutdownReason::OperatorRequest);
    let outcome = idle.execute.execute(&prepared.token).await.unwrap();
    assert!(!outcome.turn_cancelled);
    let joined = idle.execute.execute(&prepared.token).await.unwrap();
    assert_eq!(joined, outcome);
    assert_eq!(idle.cancellation.calls.load(Ordering::SeqCst), 1);
}
