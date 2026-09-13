//! A termination signal is delivered to the one teardown controller (#1938):
//! the common shutdown settles the fleet — a script-managed member's
//! environment kill included — before the dispatch loop is told to exit.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};
use crate::domain::subagent_teardown::LaunchGeneration;
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, SubagentRegistry};
use crate::interface::cli::uds_cancel::{CancelHandle, CancelSlot, TurnControl};
use crate::interface::cli::uds_teardown_graph::TeardownGraphInputs;
use crate::interface::uds::subagent_teardown::controller::ControllerOutcome;

fn environment_with_kill_script(
    dir: &std::path::Path,
    member: &str,
) -> (EnvironmentRegistry, String) {
    let script = dir.join("kill.sh");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf '%s\\n' \"$QUECTO_CONTAINER_ENVIRONMENT_ID\" > \"$(dirname \"$0\")/killed\"\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    let registry = EnvironmentRegistry::new();
    let env_ref = registry.mint_ref();
    registry.commit(EnvironmentRecord {
        environment_ref: env_ref.clone(),
        environment_id: "env-termination".to_string(),
        environment_uuid: "uuid-termination".to_string(),
        name: None,
        workspace_path: dir.to_path_buf(),
        repository: String::new(),
        script_name: "default".to_string(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec![script.display().to_string()],
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members: vec![member.to_string()],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
    });
    (registry, env_ref)
}

struct Rig {
    subagents: SubagentRegistry,
    cancel: CancelHandle,
    notify: Arc<tokio::sync::Notify>,
    busy: super::super::uds_multi::BusyFlag,
    graph: super::super::uds_teardown_graph::TeardownGraph,
}

/// The composed graph over a registry holding one script-managed member
/// this harness launched (a launch generation, no owned process, a socket
/// that never existed).
fn rig(dir: &std::path::Path) -> Rig {
    let (environments, env_ref) = environment_with_kill_script(dir, "worker");
    let subagents: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let mut entry = SubagentEntry::with_identity(
        crate::domain::ids::AgentUuid::new("worker"),
        "worker".into(),
        dir.join("never.sock"),
        0,
    );
    entry.launch_generation = Some(LaunchGeneration::new(1));
    entry.environment_registry = Some(environments);
    entry.environment_ref = Some(env_ref);
    subagents.lock().unwrap().insert("worker".into(), entry);
    let cancel: CancelHandle = Arc::new(Mutex::new(CancelSlot::Idle));
    let notify = Arc::new(tokio::sync::Notify::new());
    let busy = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let graph = crate::composition::subagent_teardown::build_teardown_graph(TeardownGraphInputs {
        owner: crate::domain::ids::AgentUuid::new("root"),
        registry: Some(subagents.clone()),
        harness_lifecycle: None,
        broadcast_tx: None,
        notify_tx: None,
        cancel_handle: cancel.clone(),
        turn_control: Arc::new(TurnControl::default()),
        busy: busy.clone(),
        exit_notify: notify.clone(),
        binding: crate::domain::parent_control::ParentControlBinding::unlaunched(),
    });
    Rig {
        subagents,
        cancel,
        notify,
        busy,
        graph,
    }
}

async fn bounded<F: std::future::Future>(future: F) -> F::Output {
    tokio::time::timeout(Duration::from_secs(20), future)
        .await
        .expect("bounded await timed out")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn termination_runs_the_common_shutdown_and_environment_kill_then_requests_loop_exit() {
    let dir = tempfile::tempdir().unwrap();
    let rig = rig(dir.path());

    let outcome = bounded(super::shutdown_on(
        std::future::ready(()),
        Some(rig.graph.controller.clone()),
        rig.busy.clone(),
        rig.cancel.clone(),
        rig.notify.clone(),
    ))
    .await
    .expect("a controller ran the shutdown");
    let ControllerOutcome::ShutdownExecuted { outcome, .. } = outcome else {
        panic!("unexpected outcome: {outcome:?}");
    };
    let outcome = outcome.unwrap();
    assert_eq!(
        outcome.reason,
        crate::domain::subagent_teardown::ShutdownReason::TerminationSignal
    );
    assert_eq!(
        outcome.children_shut_down,
        [crate::domain::ids::AgentUuid::new("worker")]
    );
    assert!(outcome.children_failed.is_empty());
    assert!(outcome.exit_signalled);
    assert!(
        rig.subagents.lock().unwrap().is_empty(),
        "the member settled and its tombstone was pruned"
    );
    let killed = std::fs::read_to_string(dir.path().join("killed"))
        .expect("the environment's retained kill argv must run on a termination signal");
    assert_eq!(killed.trim(), "env-termination");
    assert!(
        matches!(*rig.cancel.lock().unwrap(), CancelSlot::Fired),
        "an in-flight turn must be cancelled so the loop can exit"
    );
    assert!(
        rig.graph.exit.signalled().is_some(),
        "composition was told the harness may exit"
    );
    assert_eq!(
        rig.graph.persistence.recorded_reason(),
        Some(crate::domain::subagent_teardown::ShutdownReason::TerminationSignal)
    );
    // The loop is told to finish; the permit is retained for a late waiter.
    bounded(rig.notify.notified()).await;
}

#[tokio::test]
async fn no_signal_means_no_teardown() {
    let dir = tempfile::tempdir().unwrap();
    let rig = rig(dir.path());
    let never = std::future::pending::<()>();
    let watcher = tokio::spawn(super::shutdown_on(
        never,
        Some(rig.graph.controller.clone()),
        rig.busy.clone(),
        rig.cancel.clone(),
        rig.notify.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!watcher.is_finished());
    assert_eq!(rig.subagents.lock().unwrap().len(), 1);
    assert!(!dir.path().join("killed").exists());
    assert!(matches!(*rig.cancel.lock().unwrap(), CancelSlot::Idle));
    assert!(rig.graph.exit.signalled().is_none());
    watcher.abort();
}

/// A loop built without a teardown graph still exits on a signal: the turn
/// is cancelled and the loop is told to finish.
#[tokio::test]
async fn without_a_controller_the_signal_cancels_and_requests_exit() {
    let cancel: CancelHandle = Arc::new(Mutex::new(CancelSlot::Idle));
    let notify = Arc::new(tokio::sync::Notify::new());
    let outcome = super::shutdown_on(
        std::future::ready(()),
        None,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
        cancel.clone(),
        notify.clone(),
    )
    .await;
    assert!(outcome.is_none());
    assert!(matches!(*cancel.lock().unwrap(), CancelSlot::Fired));
    bounded(notify.notified()).await;
}

/// The simulated signal drives the installed watcher exactly like SIGTERM.
#[tokio::test]
async fn a_simulated_termination_signal_reaches_the_installed_watcher() {
    let cancel: CancelHandle = Arc::new(Mutex::new(CancelSlot::Idle));
    let notify = Arc::new(tokio::sync::Notify::new());
    let request = super::ShutdownRequest::install(
        notify.clone(),
        None,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
        cancel.clone(),
    );
    // The watcher must be parked on the signal before it is delivered.
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
    super::test_support::deliver_termination_signal();
    bounded(request.requested()).await;
    assert!(matches!(*cancel.lock().unwrap(), CancelSlot::Fired));
    // Without a controller the last client's disconnect has nothing to run.
    assert!(request.last_client_disconnected().await.is_none());
}

/// The last client of the default lifetime runs the same common shutdown
/// through the controller, to completion, before the loop returns.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_last_client_disconnect_runs_the_common_shutdown_to_completion() {
    let dir = tempfile::tempdir().unwrap();
    let rig = rig(dir.path());
    let request = super::ShutdownRequest::install(
        rig.notify.clone(),
        Some(rig.graph.controller.clone()),
        rig.busy.clone(),
        rig.cancel.clone(),
    );
    let outcome = bounded(request.last_client_disconnected())
        .await
        .expect("a controller ran the shutdown");
    let ControllerOutcome::ShutdownExecuted { outcome, .. } = outcome else {
        panic!("unexpected outcome: {outcome:?}");
    };
    let outcome = outcome.unwrap();
    assert_eq!(
        outcome.reason,
        crate::domain::subagent_teardown::ShutdownReason::OperatorRequest
    );
    assert_eq!(
        outcome.triggers,
        [crate::application::subagents::dto::ShutdownTrigger::LastClientDisconnected]
    );
    assert!(rig.subagents.lock().unwrap().is_empty());
    assert!(dir.path().join("killed").exists());
    assert!(
        !rig.graph.transaction.accepts_new_work(),
        "the harness is terminated: no spawn is admitted"
    );
}
