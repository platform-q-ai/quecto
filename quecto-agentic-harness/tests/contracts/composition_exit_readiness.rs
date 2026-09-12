//! Contract for [`CompositionExitReadiness`] (#1934): the application
//! signals readiness exactly once, last, after persistence and lifecycle
//! termination; composition owns the actual process exit.
use std::sync::Arc;

use quecto::application::subagents::dto::HarnessShutdownError;
use quecto::application::subagents::ports::{
    CompositionExitReadiness, SubagentLifecycleRepository,
};
use quecto::domain::subagent_teardown::{HarnessLifecycleState, ShutdownReason};

use super::teardown_fixture::{Exit, Harness, root_tree};

#[tokio::test]
async fn port_is_object_safe_and_records_the_reason() {
    let exit = Arc::new(Exit::default());
    let port: Arc<dyn CompositionExitReadiness> = exit.clone();
    port.signal_exit_ready(ShutdownReason::TerminationSignal)
        .await;
    assert_eq!(
        *exit.signalled.lock().unwrap(),
        [ShutdownReason::TerminationSignal]
    );
}

#[tokio::test]
async fn readiness_is_signalled_once_after_termination_and_shared_by_joiners() {
    let harness = Harness::new(root_tree());
    let prepared = harness.prepared(ShutdownReason::OperatorRequest);
    assert!(harness.exit.signalled.lock().unwrap().is_empty());
    let outcome = harness.execute.execute(&prepared.token).await.unwrap();
    assert!(outcome.exit_signalled);
    assert_eq!(
        harness.lifecycle.lifecycle(),
        HarnessLifecycleState::Terminated
    );
    assert_eq!(
        *harness.exit.signalled.lock().unwrap(),
        [ShutdownReason::OperatorRequest]
    );
    let joined = harness.execute.execute(&prepared.token).await.unwrap();
    assert_eq!(joined, outcome);
    assert_eq!(harness.exit.signalled.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn readiness_is_never_signalled_without_an_admitted_token() {
    let harness = Harness::new(root_tree());
    let foreign = Harness::new(root_tree()).prepared(ShutdownReason::OperatorRequest);
    assert_eq!(
        harness.execute.execute(&foreign.token).await,
        Err(HarnessShutdownError::NotPrepared)
    );
    assert!(harness.exit.signalled.lock().unwrap().is_empty());
}
