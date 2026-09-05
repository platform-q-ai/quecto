//! A termination signal must tear down container environments — which live
//! outside the harness process group — before the process exits, and must
//! then wake the dispatch loop so the session persists an empty roster.
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, SubagentRegistry};
use crate::interface::cli::uds_cancel::{CancelHandle, CancelSlot};

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

#[tokio::test]
async fn termination_runs_environment_kill_then_requests_loop_exit() {
    let dir = tempfile::tempdir().unwrap();
    let (environments, env_ref) = environment_with_kill_script(dir.path(), "worker");
    let subagents: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/worker.sock"), 0);
    entry.environment_registry = Some(environments.clone());
    entry.environment_ref = Some(env_ref.clone());
    subagents.lock().unwrap().insert("worker".into(), entry);
    let cancel: CancelHandle = Arc::new(Mutex::new(CancelSlot::Idle));
    let notify = Arc::new(tokio::sync::Notify::new());

    let removed = super::shutdown_on(
        std::future::ready(()),
        subagents.clone(),
        None,
        cancel.clone(),
        notify.clone(),
    )
    .await;

    assert_eq!(removed, 1);
    assert!(subagents.lock().unwrap().is_empty());
    let killed = std::fs::read_to_string(dir.path().join("killed"))
        .expect("the environment's retained kill argv must run on a termination signal");
    assert_eq!(killed.trim(), "env-termination");
    let record = environments
        .get(&env_ref)
        .expect("record kept for listings");
    assert_eq!(record.status, EnvironmentStatus::Stopped);
    assert!(
        matches!(*cancel.lock().unwrap(), CancelSlot::Fired),
        "an in-flight turn must be cancelled so the loop can exit"
    );
    tokio::time::timeout(std::time::Duration::from_millis(200), notify.notified())
        .await
        .expect("the loop-exit request must be retained until the loop waits on it");
}

#[tokio::test]
async fn no_signal_means_no_teardown() {
    let subagents: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    subagents.lock().unwrap().insert(
        "worker".into(),
        SubagentEntry::new(PathBuf::from("/tmp/worker.sock"), 0),
    );
    let cancel: CancelHandle = Arc::new(Mutex::new(CancelSlot::Idle));
    let notify = Arc::new(tokio::sync::Notify::new());
    let pending = super::shutdown_on(
        std::future::pending::<()>(),
        subagents.clone(),
        None,
        cancel.clone(),
        notify,
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), pending)
            .await
            .is_err()
    );
    assert_eq!(subagents.lock().unwrap().len(), 1);
    assert!(matches!(*cancel.lock().unwrap(), CancelSlot::Idle));
}
