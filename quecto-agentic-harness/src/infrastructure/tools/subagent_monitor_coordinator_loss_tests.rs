//! #1924: a `connection_closed` death signal for the coordinator of a running
//! swarm must not cascade into environment destruction. Through the
//! production death-signal entry point, against a real coordination store
//! and a real (logging) retained kill script: the environment record stays,
//! no kill argv runs, and the run is paused holding `failed` with a resume
//! blocker naming the lost coordinator. A plain container child (bootstrap
//! placeholder run) keeps the ordinary final-member kill.

use super::*;
use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus, mint_environment_uuid,
};
use crate::infrastructure::tools::swarm_bridge::SwarmContext;
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn write_kill_script(dir: &std::path::Path, log: &std::path::Path) -> std::path::PathBuf {
    let script = dir.join("kill.sh");
    std::fs::write(
        &script,
        format!(
            "#!/usr/bin/env bash\necho \"kill ${{QUECTO_CONTAINER_ENVIRONMENT_ID:-}}\" >> '{}'\n",
            log.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    script
}

fn store_context(checkout: &std::path::Path) -> SwarmContext {
    SwarmContext {
        lifecycle: Arc::new(crate::application::swarm::LifecycleService),
        checkout: checkout.to_path_buf(),
        member: "coordinator".into(),
    }
}

/// A store whose run was created by "coordinator" and is running.
fn create_running_swarm(checkout: &std::path::Path) -> SwarmContext {
    let context = store_context(checkout);
    std::fs::create_dir_all(checkout.join(".quecto")).unwrap();
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 600;
    context
        .call(
            "create",
            json!(["ship", [], [{"id":"tests","kind":"command","description":"pass"}], 3, deadline]),
        )
        .unwrap();
    context
}

/// A container that never created a run: the bootstrap placeholder only.
fn bootstrap_placeholder(checkout: &std::path::Path) -> SwarmContext {
    let context = store_context(checkout);
    std::fs::create_dir_all(checkout.join(".quecto")).unwrap();
    context
        .call("_bootstrap", json!([std::process::id(), "1", null]))
        .unwrap();
    context
}

fn environment(
    kill: std::path::PathBuf,
    checkout: &std::path::Path,
) -> (EnvironmentRegistry, String) {
    let environments = EnvironmentRegistry::new();
    let env_ref = environments.mint_ref();
    environments.commit(EnvironmentRecord {
        environment_ref: env_ref.clone(),
        environment_id: "env-coordinator".into(),
        environment_uuid: mint_environment_uuid(),
        name: None,
        workspace_path: checkout.to_path_buf(),
        repository: String::new(),
        script_name: "default".into(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec![kill.display().to_string()],
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members: vec![],
        status: EnvironmentStatus::Running,
        metadata: json!({ "checkout": checkout.display().to_string() }),
        last_error: None,
    });
    (environments, env_ref)
}

fn register_member(
    dir: &std::path::Path,
    environments: &EnvironmentRegistry,
    env_ref: &str,
) -> SubagentRegistry {
    environments.add_member(env_ref, "coordinator-1").unwrap();
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let mut entry = SubagentEntry::new(dir.join("coordinator-1.sock"), 0);
    entry.environment_registry = Some(environments.clone());
    entry.environment_ref = Some(env_ref.to_string());
    registry
        .lock()
        .unwrap()
        .insert("coordinator-1".to_string(), entry);
    registry
}

#[tokio::test]
async fn coordinator_connection_closed_retains_environment_and_pauses_run() {
    let dir = tempfile::tempdir().unwrap();
    let checkout = dir.path().join("checkout");
    let context = create_running_swarm(&checkout);
    let log = dir.path().join("kill-log.txt");
    let kill = write_kill_script(dir.path(), &log);
    let (environments, env_ref) = environment(kill, &checkout);
    let registry = register_member(dir.path(), &environments, &env_ref);

    notify_child_exited(
        &registry,
        "coordinator-1",
        &test_observer(&registry),
        crate::application::subagents::ports::ExitObservation::ConnectionClosed,
    )
    .await;

    assert!(
        !log.exists(),
        "the retained kill must not run for a lost coordinator: {:?}",
        std::fs::read_to_string(&log)
    );
    let record = environments.get(&env_ref).expect("environment record kept");
    assert_eq!(record.status, EnvironmentStatus::Retained, "{record:?}");
    assert!(record.members.is_empty(), "the member is still marked gone");
    assert!(
        record.metadata["retained"]
            .as_str()
            .unwrap()
            .contains("coordinator"),
        "{record:?}"
    );
    assert_eq!(
        registry.lock().unwrap()["coordinator-1"].status,
        SubagentStatus::Exited
    );
    assert!(context.database().is_file(), "the store survives");
    let summary = context.summary().unwrap();
    assert_eq!(summary["status"], "paused", "{summary}");
    assert_eq!(summary["outcome"], "failed", "{summary}");
    let receipt = context.control_status().unwrap();
    let blockers = receipt["resume_blockers"].as_array().unwrap();
    assert!(
        blockers
            .iter()
            .any(|b| b.as_str().unwrap().contains("'coordinator'")),
        "resume blockers must name the lost coordinator: {receipt}"
    );
    // A supervisor resume through the port refuses while the coordinator is lost.
    let error = context.resume_external().unwrap_err().to_string();
    assert!(error.contains("lost coordinator"), "{error}");
    // Only an explicit kill_container may run the retained kill now.
    assert!(environments.begin_kill(&env_ref).is_ok());
}

#[tokio::test]
async fn plain_container_child_connection_closed_keeps_the_final_member_kill() {
    let dir = tempfile::tempdir().unwrap();
    let checkout = dir.path().join("checkout");
    let _placeholder = bootstrap_placeholder(&checkout);
    let log = dir.path().join("kill-log.txt");
    let kill = write_kill_script(dir.path(), &log);
    let (environments, env_ref) = environment(kill, &checkout);
    let registry = register_member(dir.path(), &environments, &env_ref);

    notify_child_exited(
        &registry,
        "coordinator-1",
        &test_observer(&registry),
        crate::application::subagents::ports::ExitObservation::ConnectionClosed,
    )
    .await;

    let text = std::fs::read_to_string(&log).unwrap_or_default();
    assert_eq!(text.trim(), "kill env-coordinator", "{text:?}");
    let record = environments.get(&env_ref).unwrap();
    assert_eq!(record.status, EnvironmentStatus::Stopped);
}

#[tokio::test]
async fn environment_without_a_store_keeps_the_final_member_kill() {
    let dir = tempfile::tempdir().unwrap();
    let checkout = dir.path().join("checkout");
    std::fs::create_dir_all(&checkout).unwrap();
    let log = dir.path().join("kill-log.txt");
    let kill = write_kill_script(dir.path(), &log);
    let (environments, env_ref) = environment(kill, &checkout);
    let registry = register_member(dir.path(), &environments, &env_ref);

    notify_child_exited(
        &registry,
        "coordinator-1",
        &test_observer(&registry),
        crate::application::subagents::ports::ExitObservation::ConnectionClosed,
    )
    .await;

    assert_eq!(
        std::fs::read_to_string(&log).unwrap_or_default().trim(),
        "kill env-coordinator"
    );
    assert_eq!(
        environments.get(&env_ref).unwrap().status,
        EnvironmentStatus::Stopped
    );
}

#[tokio::test]
async fn coordinator_connection_closed_after_an_orderly_end_retains_without_a_loss() {
    let dir = tempfile::tempdir().unwrap();
    let checkout = dir.path().join("checkout");
    let context = create_running_swarm(&checkout);
    context
        .call("stop", json!(["blocked", "needs the master"]))
        .unwrap();
    let log = dir.path().join("kill-log.txt");
    let kill = write_kill_script(dir.path(), &log);
    let (environments, env_ref) = environment(kill, &checkout);
    let registry = register_member(dir.path(), &environments, &env_ref);

    notify_child_exited(
        &registry,
        "coordinator-1",
        &test_observer(&registry),
        crate::application::subagents::ports::ExitObservation::ConnectionClosed,
    )
    .await;

    assert!(!log.exists(), "every swarm end keeps its container");
    let record = environments.get(&env_ref).unwrap();
    assert_eq!(record.status, EnvironmentStatus::Retained);
    let reason = record.metadata["retained"].as_str().unwrap();
    assert!(reason.starts_with("run ended: blocked"), "{reason}");
    let summary = context.summary().unwrap();
    assert_eq!(
        summary["outcome"], "blocked",
        "the held outcome is untouched"
    );
    let receipt = context.control_status().unwrap();
    assert_eq!(
        receipt["resume_blockers"],
        json!([]),
        "an orderly end adds no lost-coordinator blocker: {receipt}"
    );
    // The supervisor can still resume the run through the port.
    context.resume_external().unwrap();
    assert_eq!(context.summary().unwrap()["status"], "running");
}

#[tokio::test]
async fn unreadable_store_maps_to_unreadable_and_retains_the_environment() {
    // The infrastructure port must turn a store that exists but cannot be
    // opened into `Unreadable`, which retains: a destroyed box is unrecoverable.
    let dir = tempfile::tempdir().unwrap();
    let checkout = dir.path().join("checkout");
    std::fs::create_dir_all(checkout.join(".quecto")).unwrap();
    std::fs::write(checkout.join(".quecto/swarm.sqlite"), b"not a database").unwrap();
    let log = dir.path().join("kill-log.txt");
    let kill = write_kill_script(dir.path(), &log);
    let (environments, env_ref) = environment(kill, &checkout);
    let registry = register_member(dir.path(), &environments, &env_ref);

    notify_child_exited(
        &registry,
        "coordinator-1",
        &test_observer(&registry),
        crate::application::subagents::ports::ExitObservation::ConnectionClosed,
    )
    .await;

    assert!(!log.exists(), "no kill for an unreadable store");
    let record = environments.get(&env_ref).unwrap();
    assert_eq!(record.status, EnvironmentStatus::Retained);
    let reason = record.metadata["retained"].as_str().unwrap();
    assert!(reason.contains("could not be read"), "{reason}");
    assert!(environments.begin_kill(&env_ref).is_ok());
}

/// A record whose advertised checkout is not an absolute path under its
/// workspace (or is missing) opens no store: the ordinary kill runs even
/// though a running swarm store exists at the advertised location.
async fn rejected_checkout_keeps_the_final_member_kill(
    advertised: impl FnOnce(&std::path::Path) -> String,
) {
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = dir.path().join("elsewhere");
    let _live = create_running_swarm(&elsewhere);
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    let log = dir.path().join("kill-log.txt");
    let kill = write_kill_script(dir.path(), &log);
    let (environments, env_ref) = environment(kill, &workspace);
    {
        let mut record = environments.get(&env_ref).unwrap();
        record.metadata = json!({ "checkout": advertised(&elsewhere) });
        environments.commit(record);
    }
    let registry = register_member(dir.path(), &environments, &env_ref);

    notify_child_exited(
        &registry,
        "coordinator-1",
        &test_observer(&registry),
        crate::application::subagents::ports::ExitObservation::ConnectionClosed,
    )
    .await;

    assert_eq!(
        std::fs::read_to_string(&log).unwrap_or_default().trim(),
        "kill env-coordinator",
        "a rejected checkout is no store"
    );
    assert_eq!(
        environments.get(&env_ref).unwrap().status,
        EnvironmentStatus::Stopped
    );
}

#[tokio::test]
async fn checkout_outside_the_workspace_is_not_opened() {
    rejected_checkout_keeps_the_final_member_kill(|elsewhere| elsewhere.display().to_string())
        .await;
}

#[tokio::test]
async fn relative_checkout_is_not_opened() {
    rejected_checkout_keeps_the_final_member_kill(|_| "checkout".to_string()).await;
}

#[tokio::test]
async fn checkout_escaping_the_workspace_with_dotdot_is_not_opened() {
    rejected_checkout_keeps_the_final_member_kill(|elsewhere| {
        // `<workspace>/../elsewhere` resolves to the live store but is refused.
        format!(
            "{}/../elsewhere",
            elsewhere.parent().unwrap().join("workspace").display()
        )
    })
    .await;
}

#[tokio::test]
async fn checkout_under_the_workspace_is_opened() {
    // The docker adapter's `--repo` layout: checkout = <workspace>/repo.
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    let checkout = workspace.join("repo");
    let _context = create_running_swarm(&checkout);
    let log = dir.path().join("kill-log.txt");
    let kill = write_kill_script(dir.path(), &log);
    let (environments, env_ref) = environment(kill, &workspace);
    {
        let mut record = environments.get(&env_ref).unwrap();
        record.metadata = json!({ "checkout": checkout.display().to_string() });
        environments.commit(record);
    }
    let registry = register_member(dir.path(), &environments, &env_ref);

    notify_child_exited(
        &registry,
        "coordinator-1",
        &test_observer(&registry),
        crate::application::subagents::ports::ExitObservation::ConnectionClosed,
    )
    .await;

    assert!(!log.exists());
    assert_eq!(
        environments.get(&env_ref).unwrap().status,
        EnvironmentStatus::Retained
    );
}

#[tokio::test]
async fn run_control_receipt_decodes_the_lost_coordinator_blocker() {
    use crate::domain::swarm::{RunControlAction, RunStatus, SwarmRunControl};
    let dir = tempfile::tempdir().unwrap();
    let checkout = dir.path().join("checkout");
    let context = create_running_swarm(&checkout);
    let store = crate::infrastructure::tools::swarm_bridge::HostedStore::at(checkout.clone());
    let hosted = store.hosted_run().unwrap().unwrap();
    assert_eq!((hosted.status, hosted.outcome), (RunStatus::Running, None));
    store.record_lost_coordinator("coordinator").unwrap();

    // Through the same port the parent's `swarm_control status` uses.
    let receipt = context.apply(RunControlAction::Status).await.unwrap();
    assert_eq!(receipt.status, RunStatus::Paused);
    assert_eq!(receipt.outcome, Some(RunStatus::Failed));
    assert_eq!(receipt.resume_blockers.len(), 1, "{receipt:?}");
    assert!(
        receipt.resume_blockers[0].contains("relaunch the lost coordinator 'coordinator'"),
        "{receipt:?}"
    );
    // The host-side observation decodes the held outcome as well.
    let hosted = store.hosted_run().unwrap().unwrap();
    assert_eq!(
        (hosted.status, hosted.outcome),
        (RunStatus::Paused, Some(RunStatus::Failed))
    );
}

// ── round three: every end retains; supervisor kills retain; probing ────

/// Drive the production death signal and return the retention reason (or
/// `None` when the record was stopped).
fn test_observer(
    registry: &SubagentRegistry,
) -> std::sync::Arc<crate::application::subagents::use_cases::ObserveOwnedChildExit> {
    super::super::subagent_teardown_wiring::build_lifecycle_use_cases(registry.clone(), None, None)
        .observe_exit
}

async fn exit_and_reason(
    environments: &EnvironmentRegistry,
    env_ref: &str,
    registry: &SubagentRegistry,
) -> Option<String> {
    notify_child_exited(
        registry,
        "coordinator-1",
        &test_observer(registry),
        crate::application::subagents::ports::ExitObservation::ConnectionClosed,
    )
    .await;
    let record = environments.get(env_ref).unwrap();
    (record.status == EnvironmentStatus::Retained)
        .then(|| record.metadata["retained"].as_str().unwrap().to_string())
}

#[tokio::test]
async fn coordinator_connection_closed_after_close_retains_the_closed_run() {
    let dir = tempfile::tempdir().unwrap();
    let checkout = dir.path().join("checkout");
    let context = create_running_swarm(&checkout);
    context.call("stop", json!(["blocked", "done"])).unwrap();
    context.close().unwrap();
    assert_eq!(context.summary().unwrap()["status"], "blocked");
    let log = dir.path().join("kill-log.txt");
    let kill = write_kill_script(dir.path(), &log);
    let (environments, env_ref) = environment(kill, &checkout);
    let registry = register_member(dir.path(), &environments, &env_ref);

    let reason = exit_and_reason(&environments, &env_ref, &registry)
        .await
        .expect("a closed run keeps its container");
    assert!(!log.exists(), "no kill after close");
    assert!(reason.starts_with("run closed: blocked"), "{reason}");
    assert_eq!(context.summary().unwrap()["status"], "blocked", "untouched");
}

#[tokio::test]
async fn coordinator_connection_closed_after_cancel_retains_the_cancelled_run() {
    let dir = tempfile::tempdir().unwrap();
    let checkout = dir.path().join("checkout");
    let context = create_running_swarm(&checkout);
    context
        .call("stop", json!(["cancelled", "operator"]))
        .unwrap();
    assert_eq!(context.summary().unwrap()["status"], "cancelled");
    let log = dir.path().join("kill-log.txt");
    let kill = write_kill_script(dir.path(), &log);
    let (environments, env_ref) = environment(kill, &checkout);
    let registry = register_member(dir.path(), &environments, &env_ref);

    let reason = exit_and_reason(&environments, &env_ref, &registry)
        .await
        .expect("a cancelled run keeps its container");
    assert!(!log.exists(), "no kill after cancel");
    assert!(reason.starts_with("run ended: cancelled"), "{reason}");
}

#[tokio::test]
async fn expired_deadline_is_observed_before_the_loss_is_recorded() {
    // The run looks `running` until a store operation notices the passed
    // deadline; the single-operation loss record must see the expiry first
    // and report an orderly budget-exhausted end, never a lost coordinator.
    let dir = tempfile::tempdir().unwrap();
    let checkout = dir.path().join("checkout");
    let context = create_running_swarm(&checkout);
    let status = std::process::Command::new("python3")
        .args([
            "-c",
            "import sqlite3,sys; db=sqlite3.connect(sys.argv[1]); db.execute('UPDATE run SET deadline=1'); db.commit()",
        ])
        .arg(context.database())
        .status()
        .unwrap();
    assert!(status.success());
    let store = crate::infrastructure::tools::swarm_bridge::HostedStore::at(checkout.clone());
    assert_eq!(
        store.hosted_run().unwrap().unwrap().status,
        crate::domain::swarm::RunStatus::Running,
        "the membership-free status read does not run the expiry check"
    );
    let log = dir.path().join("kill-log.txt");
    let kill = write_kill_script(dir.path(), &log);
    let (environments, env_ref) = environment(kill, &checkout);
    let registry = register_member(dir.path(), &environments, &env_ref);

    let reason = exit_and_reason(&environments, &env_ref, &registry)
        .await
        .expect("retained");
    assert!(
        reason.starts_with("run ended: budget-exhausted"),
        "{reason}"
    );
    let summary = context.summary().unwrap();
    assert_eq!(summary["outcome"], "budget-exhausted", "{summary}");
    let receipt = context.control_status().unwrap();
    let blockers = receipt["resume_blockers"].as_array().unwrap();
    assert!(
        blockers
            .iter()
            .all(|b| !b.as_str().unwrap().contains("coordinator")),
        "no lost-coordinator blocker after an orderly expiry: {receipt}"
    );
}

#[tokio::test]
async fn supervisor_kill_of_the_coordinator_retains_the_environment() {
    // agent_cmd kill of the member: ParentKill through the shared cascade
    // teardown, exactly as agent_cmd.rs and the master's shutdown call it.
    let dir = tempfile::tempdir().unwrap();
    let checkout = dir.path().join("checkout");
    let context = create_running_swarm(&checkout);
    context
        .call("stop", json!(["blocked", "for review"]))
        .unwrap();
    let log = dir.path().join("kill-log.txt");
    let kill = write_kill_script(dir.path(), &log);
    let (environments, env_ref) = environment(kill, &checkout);
    let registry = register_member(dir.path(), &environments, &env_ref);

    let super::super::subagent_cascade::CascadeOutcome { removed, .. } =
        super::super::subagent_cascade::cascade_remove_and_state_changed(
            &registry,
            "coordinator-1",
        );
    let mut removed = removed;
    super::super::subagent_cleanup::cleanup_removed_entries_once(
        &mut removed,
        super::super::subagent_cleanup::FinalizeMode::ParentKill,
    )
    .await;

    assert!(
        !log.exists(),
        "a supervisor kill of the coordinator keeps the box"
    );
    let record = environments.get(&env_ref).unwrap();
    assert_eq!(record.status, EnvironmentStatus::Retained);
    let reason = record.metadata["retained"].as_str().unwrap();
    assert!(
        reason.starts_with("coordinator killed by supervisor; run paused holding blocked"),
        "{reason}"
    );
    assert_eq!(
        context.summary().unwrap()["outcome"],
        "blocked",
        "no quarantine"
    );
    assert!(environments.begin_kill(&env_ref).is_ok());
}

#[tokio::test]
async fn master_shutdown_teardown_retains_a_swarm_environment() {
    // The synchronous process-shutdown path (uds_shutdown run_teardown ->
    // teardown_all -> cleanup_removed_entries_sync).
    let dir = tempfile::tempdir().unwrap();
    let checkout = dir.path().join("checkout");
    let _context = create_running_swarm(&checkout);
    let log = dir.path().join("kill-log.txt");
    let kill = write_kill_script(dir.path(), &log);
    let (environments, env_ref) = environment(kill, &checkout);
    let registry = register_member(dir.path(), &environments, &env_ref);

    let mut removed: Vec<(String, SubagentEntry)> = registry.lock().unwrap().drain().collect();
    tokio::task::spawn_blocking(move || {
        super::super::subagent_cleanup::cleanup_removed_entries_sync(&mut removed);
    })
    .await
    .unwrap();

    assert!(!log.exists(), "master shutdown keeps a swarm's box");
    let record = environments.get(&env_ref).unwrap();
    assert_eq!(record.status, EnvironmentStatus::Retained);
    assert!(
        record.metadata["retained"]
            .as_str()
            .unwrap()
            .starts_with("coordinator killed by supervisor; run running"),
        "{record:?}"
    );
}

#[tokio::test]
async fn master_shutdown_teardown_still_kills_an_ordinary_environment() {
    let dir = tempfile::tempdir().unwrap();
    let checkout = dir.path().join("checkout");
    let _placeholder = bootstrap_placeholder(&checkout);
    let log = dir.path().join("kill-log.txt");
    let kill = write_kill_script(dir.path(), &log);
    let (environments, env_ref) = environment(kill, &checkout);
    let registry = register_member(dir.path(), &environments, &env_ref);
    let mut removed: Vec<(String, SubagentEntry)> = registry.lock().unwrap().drain().collect();
    tokio::task::spawn_blocking(move || {
        super::super::subagent_cleanup::cleanup_removed_entries_sync(&mut removed);
    })
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(&log).unwrap_or_default().trim(),
        "kill env-coordinator"
    );
    assert_eq!(
        environments.get(&env_ref).unwrap().status,
        EnvironmentStatus::Stopped
    );
}

/// A create result without `metadata.checkout` (older / third-party script
/// sets): the host probes `<workspace>/repo` then `<workspace>`.
async fn probe_case(store_under: &str) {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    let checkout = if store_under.is_empty() {
        workspace.clone()
    } else {
        workspace.join(store_under)
    };
    let _context = create_running_swarm(&checkout);
    let log = dir.path().join("kill-log.txt");
    let kill = write_kill_script(dir.path(), &log);
    let (environments, env_ref) = environment(kill, &workspace);
    {
        let mut record = environments.get(&env_ref).unwrap();
        record.metadata = json!({});
        environments.commit(record);
    }
    let registry = register_member(dir.path(), &environments, &env_ref);
    let reason = exit_and_reason(&environments, &env_ref, &registry)
        .await
        .unwrap_or_else(|| panic!("store under {store_under:?} should be found by probing"));
    assert!(!log.exists());
    assert!(reason.contains("lost its connection"), "{reason}");
}

#[tokio::test]
async fn missing_checkout_probes_the_repo_subdirectory() {
    probe_case("repo").await;
}

#[tokio::test]
async fn missing_checkout_probes_the_workspace_itself() {
    probe_case("").await;
}

#[tokio::test]
async fn missing_checkout_without_a_store_keeps_the_final_member_kill() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(workspace.join("repo")).unwrap();
    let log = dir.path().join("kill-log.txt");
    let kill = write_kill_script(dir.path(), &log);
    let (environments, env_ref) = environment(kill, &workspace);
    {
        let mut record = environments.get(&env_ref).unwrap();
        record.metadata = json!({});
        environments.commit(record);
    }
    let registry = register_member(dir.path(), &environments, &env_ref);
    assert!(
        exit_and_reason(&environments, &env_ref, &registry)
            .await
            .is_none()
    );
    assert_eq!(
        std::fs::read_to_string(&log).unwrap_or_default().trim(),
        "kill env-coordinator"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_checkout_pointing_outside_the_workspace_is_not_opened() {
    let dir = tempfile::tempdir().unwrap();
    let elsewhere = dir.path().join("elsewhere");
    let _live = create_running_swarm(&elsewhere);
    let workspace = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace).unwrap();
    std::os::unix::fs::symlink(&elsewhere, workspace.join("repo")).unwrap();
    let log = dir.path().join("kill-log.txt");
    let kill = write_kill_script(dir.path(), &log);
    let (environments, env_ref) = environment(kill, &workspace);
    for metadata in [
        json!({ "checkout": workspace.join("repo").display().to_string() }),
        json!({}),
    ] {
        let mut record = environments.get(&env_ref).unwrap();
        record.status = EnvironmentStatus::Running;
        record.metadata = metadata.clone();
        environments.commit(record);
        let _ = std::fs::remove_file(&log);
        let registry = register_member(dir.path(), &environments, &env_ref);
        assert!(
            exit_and_reason(&environments, &env_ref, &registry)
                .await
                .is_none(),
            "{metadata}: a symlink out of the workspace is refused"
        );
        assert_eq!(
            std::fs::read_to_string(&log).unwrap_or_default().trim(),
            "kill env-coordinator",
            "{metadata}"
        );
    }
}
