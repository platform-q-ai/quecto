//! Contract for [`HostedSwarmRunObservation`] (#1924, #1939), proven on the
//! production store adapter: an environment without a reachable checkout
//! observes no store, a created run is observed as the run it is, and recording a
//! coordinator loss pauses a live run holding `failed` exactly once while
//! leaving an ended run alone. (The bootstrap placeholder is proven in the
//! adapter's own unit suite, which can reach the store's test entry points.)
use std::sync::Arc;

use quecto::application::environments::ports::HostedSwarmRunObservation;
use quecto::domain::environment_registry::{
    EnvironmentRecord, EnvironmentStatus, mint_environment_uuid,
};
use quecto::domain::environment_retention::SwarmRunObservation;
use quecto::domain::swarm::RunStatus;
use quecto::infrastructure::tools::environment_commands::HostedStoreObservation;
use quecto::infrastructure::tools::swarm_bridge::SwarmContext;

fn port() -> Arc<dyn HostedSwarmRunObservation + Send + Sync> {
    Arc::new(HostedStoreObservation)
}

fn record(checkout: &std::path::Path, advertise: bool) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: "C1".into(),
        environment_id: "env-contract".into(),
        environment_uuid: mint_environment_uuid(),
        name: None,
        workspace_path: checkout.to_path_buf(),
        repository: String::new(),
        script_name: "default".into(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec!["true".into()],
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members: vec![],
        status: EnvironmentStatus::Running,
        metadata: if advertise {
            serde_json::json!({ "checkout": checkout.display().to_string() })
        } else {
            serde_json::json!({})
        },
        last_error: None,
        origin: quecto::domain::environment_registry::EnvironmentOrigin::Created,
        created_by: String::new(),
        created_at: None,
    }
}

fn store(checkout: &std::path::Path) -> SwarmContext {
    std::fs::create_dir_all(checkout.join(".quecto")).unwrap();
    SwarmContext {
        checkout: checkout.to_path_buf(),
        member: "coordinator".into(),
        lifecycle: Arc::new(quecto::application::swarm::LifecycleService),
    }
}

fn create_run(context: &SwarmContext) {
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 600;
    context
        .create_run(
            &serde_json::json!({"goal":"ship", "constraints":[], "criteria":[{"id":"tests","kind":"command","description":"pass"}], "member_limit":3, "deadline":deadline}),
            &quecto::domain::swarm::ProcessIdentity {
                pid: std::process::id(),
                started: quecto::infrastructure::tools::swarm_bridge::process_start(
                    std::process::id(),
                )
                .unwrap(),
            },
            None,
        )
        .unwrap();
}

#[tokio::test]
async fn an_environment_without_a_store_observes_no_store_and_cannot_record_a_loss() {
    let temp = tempfile::tempdir().unwrap();
    let port = port();
    let record = record(temp.path(), true);
    assert_eq!(
        port.observe_hosted_swarm_run(&record).await,
        SwarmRunObservation::NoStore
    );
    // A workspace-less record has no host location a store may live at.
    let mut homeless = record.clone();
    homeless.metadata = serde_json::json!({ "checkout": "/definitely/not/here" });
    assert_eq!(
        port.observe_hosted_swarm_run(&homeless).await,
        SwarmRunObservation::NoStore
    );
    let hosted = quecto::domain::environment_retention::HostedSwarmRun {
        status: RunStatus::Running,
        outcome: None,
        coordinator: "coordinator".into(),
        deadline: 1.0,
    };
    let err = port
        .record_lost_coordinator(&homeless, &hosted)
        .await
        .unwrap_err();
    assert!(err.contains("no checkout"), "{err}");
}

#[tokio::test]
async fn a_created_run_is_observed_and_its_loss_is_recorded_exactly_once() {
    let temp = tempfile::tempdir().unwrap();
    let context = store(temp.path());
    create_run(&context);
    let port = port();
    let record = record(temp.path(), true);
    let SwarmRunObservation::Run(hosted) = port.observe_hosted_swarm_run(&record).await else {
        panic!("a created run is observed");
    };
    assert!(hosted.created());
    assert!(!hosted.ended());
    assert_eq!(hosted.status, RunStatus::Running);
    assert_eq!(hosted.coordinator, "coordinator");

    let loss = port
        .record_lost_coordinator(&record, &hosted)
        .await
        .unwrap();
    assert!(loss.lost, "{loss:?}");
    assert_eq!(loss.run.status, RunStatus::Paused);
    assert_eq!(loss.run.outcome, Some(RunStatus::Failed));

    // Recording again finds a run that already ended: left alone.
    let again = port
        .record_lost_coordinator(&record, &loss.run)
        .await
        .unwrap();
    assert!(!again.lost, "{again:?}");
    assert_eq!(again.run.status, RunStatus::Paused);

    // The un-advertised record still finds the store under its workspace.
    let SwarmRunObservation::Run(probed) = port
        .observe_hosted_swarm_run(&self::record(temp.path(), false))
        .await
    else {
        panic!("the workspace itself is probed for a store");
    };
    assert_eq!(probed.status, RunStatus::Paused);
}
