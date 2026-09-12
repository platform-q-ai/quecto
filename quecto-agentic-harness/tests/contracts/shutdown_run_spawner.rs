//! Contract for [`ShutdownRunSpawner`] (#1934): the transaction, not the
//! caller, owns the detached teardown. A dropped caller never stops it; a
//! spawner that drops the run unpolled hands the admission back so the
//! next `Execute` resumes it; effects never repeat across a re-drive.
use std::sync::Arc;
use std::sync::atomic::Ordering;

use quecto::application::subagents::dto::{HarnessShutdownError, PersistenceOutcome};
use quecto::application::subagents::ports::{ShutdownRunSpawner, SubagentLifecycleRepository};
use quecto::domain::subagent_teardown::{HarnessLifecycleState, ShutdownReason};

use super::teardown_fixture::{Harness, Spawner, root_tree};

#[tokio::test]
async fn port_is_object_safe_and_runs_a_static_future() {
    let spawner = Arc::new(Spawner::default());
    let port: Arc<dyn ShutdownRunSpawner> = spawner.clone();
    let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = flag.clone();
    port.spawn_shutdown_run(Box::pin(async move {
        seen.store(true, Ordering::SeqCst);
    }));
    spawner.latest_finished().await;
    assert!(flag.load(Ordering::SeqCst));
}

#[tokio::test]
async fn dropping_the_caller_never_stops_the_detached_run() {
    let harness = Harness::new(root_tree());
    harness.exit.hold.store(true, Ordering::SeqCst);
    let prepared = harness.prepared(ShutdownReason::ParentShutdown);
    let caller = tokio::spawn({
        let token = prepared.token.clone();
        let execute = harness.execute.clone();
        async move { execute.execute(&token).await }
    });
    while harness.exit.attempts.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    // The run is the transaction's, not the caller's: it still finishes.
    harness.exit.gate.notify_one();
    harness.spawner.latest_finished().await;
    assert_eq!(harness.exit.signalled.lock().unwrap().len(), 1);
    assert_eq!(
        harness.lifecycle.lifecycle(),
        HarnessLifecycleState::Terminated
    );
    let outcome = harness.execute.execute(&prepared.token).await.unwrap();
    assert!(outcome.exit_signalled);
}

#[tokio::test]
async fn a_dropped_run_is_resumed_by_the_next_execute_without_repeating_effects() {
    let harness = Harness::new(root_tree());
    harness.exit.hold.store(true, Ordering::SeqCst);
    let prepared = harness.prepared(ShutdownReason::ParentShutdown);
    let join = tokio::spawn({
        let token = prepared.token.clone();
        let execute = harness.execute.clone();
        async move { execute.execute(&token).await }
    });
    while harness.exit.attempts.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    // The runtime drops the detached run after persistence but before exit.
    harness.spawner.abort_latest().await;
    assert_eq!(
        join.await.unwrap(),
        Err(HarnessShutdownError::ExecutionInterrupted)
    );
    assert_eq!(harness.lifecycle.lifecycle(), HarnessLifecycleState::Frozen);
    harness.exit.hold.store(false, Ordering::SeqCst);
    let outcome = harness.execute.execute(&prepared.token).await.unwrap();
    assert_eq!(outcome.persistence, PersistenceOutcome::Persisted);
    assert!(outcome.exit_signalled);
    assert_eq!(harness.cancellation.calls.load(Ordering::SeqCst), 1);
    assert_eq!(harness.routing.calls().len(), 2);
    assert_eq!(harness.persistence.calls.lock().unwrap().len(), 1);
    assert_eq!(harness.exit.signalled.lock().unwrap().len(), 1);
    assert_eq!(harness.exit.attempts.load(Ordering::SeqCst), 2);
    assert_eq!(
        harness.lifecycle.lifecycle(),
        HarnessLifecycleState::Terminated
    );
}

#[tokio::test]
async fn a_spawner_that_drops_the_run_unpolled_reports_interruption() {
    let harness = Harness::new(root_tree());
    harness.spawner.drop_next.store(1, Ordering::SeqCst);
    let prepared = harness.prepared(ShutdownReason::OperatorRequest);
    assert_eq!(
        harness.execute.execute(&prepared.token).await,
        Err(HarnessShutdownError::ExecutionInterrupted)
    );
    assert_eq!(harness.cancellation.calls.load(Ordering::SeqCst), 0);
    assert!(harness.execute.execute(&prepared.token).await.is_ok());
    assert_eq!(harness.spawner.spawned.load(Ordering::SeqCst), 2);
}
