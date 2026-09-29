//! Contract for [`ExternalAgentSpawner`] (#2304 review round 2), proven on
//! the production `TokioExternalAgentSpawner`: the work it is handed runs
//! to its end though the caller that handed it over is gone.
use std::time::Duration;

use quecto::application::external_agent::ports::ExternalAgentSpawner;
use quecto::infrastructure::external_agents::spawner::TokioExternalAgentSpawner;

#[tokio::test]
async fn work_runs_to_its_end_after_its_caller_is_dropped() {
    let (done, finished) = tokio::sync::oneshot::channel();
    let caller = async {
        TokioExternalAgentSpawner.spawn(Box::pin(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let _ = done.send(());
        }));
        // The caller never gets further: it is dropped here.
        std::future::pending::<()>().await;
    };
    assert!(
        tokio::time::timeout(Duration::from_millis(5), caller)
            .await
            .is_err()
    );
    tokio::time::timeout(Duration::from_secs(5), finished)
        .await
        .expect("the work ran to its end")
        .expect("it told so");
}

#[tokio::test(flavor = "current_thread")]
async fn spawning_returns_at_once_on_a_single_thread() {
    let (done, finished) = tokio::sync::oneshot::channel();
    TokioExternalAgentSpawner.spawn(Box::pin(async move {
        let _ = done.send(());
    }));
    tokio::time::timeout(Duration::from_secs(5), finished)
        .await
        .expect("the work runs once the caller yields")
        .expect("it told so");
}
