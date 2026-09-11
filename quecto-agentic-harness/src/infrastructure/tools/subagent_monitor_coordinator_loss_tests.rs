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
use crate::infrastructure::tools::subagent_registry::ExitSignalKind;
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
        None,
        None,
        ExitSignalKind::ConnectionClosed,
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
        None,
        None,
        ExitSignalKind::ConnectionClosed,
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
        None,
        None,
        ExitSignalKind::ConnectionClosed,
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
