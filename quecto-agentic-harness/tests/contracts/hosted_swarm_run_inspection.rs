//! Contract for [`HostedSwarmRunInspection`] (round 4 M1, #2033), proven on
//! the production store adapter: the synchronous read the startup restore
//! and the CLI collector make of the store a checkout hosts. A record is
//! read at its checkout exactly as the finalizer's observation reads it; a
//! bare state directory no record names is probed at `workspace/repo` then
//! `workspace`; a directory hosting no store, or one whose store lies
//! outside it through a symlink, is `NoStore`; nothing is written.
use std::sync::Arc;

use quecto::application::environments::ports::HostedSwarmRunInspection;
use quecto::domain::environment_registry::{
    EnvironmentRecord, EnvironmentStatus, mint_environment_uuid,
};
use quecto::domain::environment_retention::SwarmRunObservation;
use quecto::domain::swarm::RunStatus;
use quecto::infrastructure::tools::environment_commands::HostedStoreObservation;
use quecto::infrastructure::tools::swarm_bridge::SwarmContext;

fn port() -> Arc<dyn HostedSwarmRunInspection> {
    Arc::new(HostedStoreObservation)
}

fn record(workspace: &std::path::Path) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: "C1".into(),
        environment_id: "env-inspect".into(),
        environment_uuid: mint_environment_uuid(),
        name: None,
        workspace_path: workspace.to_path_buf(),
        repository: String::new(),
        script_name: "default".into(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec!["true".into()],
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members: vec![],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
        origin: quecto::domain::environment_registry::EnvironmentOrigin::Created,
        created_by: String::new(),
        created_at: None,
    }
}

fn create_run(checkout: &std::path::Path) -> SwarmContext {
    std::fs::create_dir_all(checkout.join(".quecto")).unwrap();
    let context = SwarmContext {
        checkout: checkout.to_path_buf(),
        member: "coordinator".into(),
        lifecycle: Arc::new(quecto::application::swarm::LifecycleService),
    };
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
    context
}

#[test]
fn a_record_is_read_at_its_checkout_and_a_bare_state_dir_at_its_workspace() {
    let temp = tempfile::tempdir().unwrap();
    let state_dir = temp.path().join("env-inspect");
    let checkout = state_dir.join("workspace").join("repo");
    create_run(&checkout);
    let port = port();

    // The record's workspace is probed for `repo/` the way the finalizer's
    // observation probes it.
    let SwarmRunObservation::Run(by_record) =
        port.inspect_hosted_run(&record(&state_dir.join("workspace")))
    else {
        panic!("a created run is read through the record");
    };
    assert!(by_record.created() && !by_record.ended());
    assert_eq!(by_record.status, RunStatus::Running);
    assert_eq!(by_record.id.len(), 32, "{by_record:?}");

    // The bare state dir — no record — reads the same run.
    let SwarmRunObservation::Run(by_dir) = port.inspect_hosted_run_at(&state_dir) else {
        panic!("a created run is read below the state dir");
    };
    assert_eq!(by_dir, by_record);

    // A store directly under `workspace` is found too.
    let flat = temp.path().join("env-flat");
    create_run(&flat.join("workspace"));
    assert!(matches!(
        port.inspect_hosted_run_at(&flat),
        SwarmRunObservation::Run(_)
    ));
}

#[test]
fn a_directory_without_a_store_or_with_one_reached_only_through_a_symlink_is_no_store() {
    let temp = tempfile::tempdir().unwrap();
    let port = port();
    let empty = temp.path().join("env-empty");
    std::fs::create_dir_all(empty.join("workspace")).unwrap();
    assert_eq!(
        port.inspect_hosted_run_at(&empty),
        SwarmRunObservation::NoStore
    );
    assert_eq!(
        port.inspect_hosted_run_at(&temp.path().join("env-missing")),
        SwarmRunObservation::NoStore
    );
    assert_eq!(
        port.inspect_hosted_run(&record(&empty.join("workspace"))),
        SwarmRunObservation::NoStore
    );

    // A `workspace` symlink leading outside the state dir is not followed
    // into a store the directory does not own.
    let elsewhere = temp.path().join("elsewhere");
    create_run(&elsewhere);
    let linked = temp.path().join("env-linked");
    std::fs::create_dir_all(&linked).unwrap();
    std::os::unix::fs::symlink(&elsewhere, linked.join("workspace")).unwrap();
    assert_eq!(
        port.inspect_hosted_run_at(&linked),
        SwarmRunObservation::NoStore
    );
}

#[test]
fn an_ended_run_is_read_as_ended_and_the_read_changes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let state_dir = temp.path().join("env-ended");
    let checkout = state_dir.join("workspace");
    let context = create_run(&checkout);
    let before = std::fs::read(checkout.join(".quecto/swarm.sqlite")).unwrap();
    let port = port();
    assert!(matches!(
        port.inspect_hosted_run_at(&state_dir),
        SwarmRunObservation::Run(run) if !run.ended()
    ));
    let receipt = quecto::infrastructure::tools::swarm_bridge::HostedStore::at(checkout.clone())
        .record_lost_coordinator(&context.member)
        .unwrap();
    assert!(receipt.lost);
    let SwarmRunObservation::Run(ended) = port.inspect_hosted_run_at(&state_dir) else {
        panic!("the ended run is still read");
    };
    assert!(ended.ended(), "{ended:?}");
    assert_eq!(ended.outcome, Some(RunStatus::Failed));
    // Two reads of an ended store leave its bytes as they were.
    let after_first = std::fs::read(checkout.join(".quecto/swarm.sqlite")).unwrap();
    port.inspect_hosted_run_at(&state_dir);
    assert_eq!(
        std::fs::read(checkout.join(".quecto/swarm.sqlite")).unwrap(),
        after_first
    );
    assert_ne!(before, after_first, "the loss itself was recorded");
}
