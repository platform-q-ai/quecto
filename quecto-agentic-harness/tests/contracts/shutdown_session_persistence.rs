//! Contract for [`ShutdownSessionPersistence`] (#1934): persistence runs
//! once per shutdown with the admitted reason, after the children were
//! addressed, and a failure is reported in the outcome rather than aborting
//! the exit.
use std::sync::Arc;

use quecto::application::subagents::dto::PersistenceOutcome;
use quecto::application::subagents::ports::ShutdownSessionPersistence;
use quecto::domain::subagent_teardown::ShutdownReason;

use super::teardown_fixture::{Harness, Persistence, root_tree};

#[tokio::test]
async fn port_is_object_safe_and_reports_failures_as_strings() {
    let persistence = Arc::new(Persistence::default());
    let port: Arc<dyn ShutdownSessionPersistence> = persistence.clone();
    assert_eq!(
        port.persist_for_shutdown(ShutdownReason::OperatorRequest)
            .await,
        Ok(())
    );
    *persistence.fail_with.lock().unwrap() = Some("read-only".into());
    assert_eq!(
        port.persist_for_shutdown(ShutdownReason::OperatorRequest)
            .await,
        Err("read-only".into())
    );
}

#[tokio::test]
async fn executed_shutdown_persists_once_with_the_admitted_reason() {
    let harness = Harness::new(root_tree());
    let prepared = harness.prepared(ShutdownReason::ParentConnectionLost);
    let outcome = harness.execute.execute(&prepared.token).await.unwrap();
    assert_eq!(outcome.persistence, PersistenceOutcome::Persisted);
    assert_eq!(
        *harness.persistence.calls.lock().unwrap(),
        [ShutdownReason::ParentConnectionLost]
    );
    harness.execute.execute(&prepared.token).await.unwrap();
    assert_eq!(harness.persistence.calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn persistence_failure_is_reported_and_the_harness_still_exits() {
    let harness = Harness::new(root_tree());
    *harness.persistence.fail_with.lock().unwrap() = Some("disk full".into());
    let prepared = harness.prepared(ShutdownReason::ParentShutdown);
    let outcome = harness.execute.execute(&prepared.token).await.unwrap();
    assert_eq!(
        outcome.persistence,
        PersistenceOutcome::Failed("disk full".into())
    );
    assert!(outcome.exit_signalled);
    assert_eq!(harness.exit.signalled.lock().unwrap().len(), 1);
}
