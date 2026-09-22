//! Contract for [`RetainedEnvironmentTeardown`] (#2070): asked exactly once
//! per executed shutdown the owner announced — after the fleet has settled,
//! never for an unannounced one — and its answers are the shutdown's own.
use std::sync::Arc;
use std::sync::atomic::Ordering;

use quecto::application::subagents::ports::{OwnerExitAnnouncement, RetainedEnvironmentTeardown};
use quecto::domain::subagent_teardown::ShutdownReason;

use super::teardown_fixture::{Harness, RetainedEnvironments, root_tree};

#[tokio::test]
async fn port_is_object_safe_and_answers_per_environment() {
    let fake = Arc::new(RetainedEnvironments::default());
    *fake.answer.lock().unwrap() = vec![("C2".into(), Ok(())), ("C5".into(), Err("busy".into()))];
    let port: Arc<dyn RetainedEnvironmentTeardown> = fake.clone();
    let answer = port.end_emptied_retained().await;
    assert_eq!(
        answer,
        [
            ("C2".to_string(), Ok(())),
            ("C5".to_string(), Err("busy".to_string()))
        ]
    );
    assert_eq!(fake.asked.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_announced_exit_asks_once_after_the_fleet_and_reports_the_answers() {
    let harness = Harness::new(root_tree());
    *harness.retained.answer.lock().unwrap() = vec![("C2".into(), Ok(()))];
    harness.owner_exit.announce(1);
    let prepared = harness.prepared(ShutdownReason::TerminationSignal);
    let outcome = harness.execute.execute(&prepared.token).await.unwrap();
    assert_eq!(outcome.retained_environments, [("C2".to_string(), Ok(()))]);
    assert_eq!(harness.retained.asked.load(Ordering::SeqCst), 1);
    assert!(
        !outcome.children_shut_down.is_empty(),
        "the fleet settled first"
    );
    // A joined re-drive reports the same answers and asks no second time.
    let joined = harness.execute.execute(&prepared.token).await.unwrap();
    assert_eq!(joined, outcome);
    assert_eq!(harness.retained.asked.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn an_unannounced_shutdown_never_asks() {
    for reason in [
        ShutdownReason::TerminationSignal,
        ShutdownReason::ParentConnectionLost,
        ShutdownReason::OperatorRequest,
    ] {
        let harness = Harness::new(root_tree());
        let prepared = harness.prepared(reason);
        let outcome = harness.execute.execute(&prepared.token).await.unwrap();
        assert!(outcome.retained_environments.is_empty(), "{reason:?}");
        assert_eq!(
            harness.retained.asked.load(Ordering::SeqCst),
            0,
            "{reason:?}"
        );
    }
}
