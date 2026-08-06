use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::subagent_cleanup::cleanup_registered_once;
use super::subagent_registry::{SubagentEntry, SubagentRegistry};

fn cleanup_script(log: &std::path::Path) -> std::path::PathBuf {
    let script = log.parent().unwrap().join("cleanup.sh");
    std::fs::write(
        &script,
        format!(
            "#!/usr/bin/env bash\necho \"$QUECTO_CONTAINER_ENVIRONMENT_ID\" >> '{}'\n",
            log.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(&script).unwrap().permissions();
        p.set_mode(0o700);
        std::fs::set_permissions(&script, p).unwrap();
    }
    script
}

#[tokio::test]
async fn cleanup_registered_once_is_claimed_by_single_concurrent_owner() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("cleanup.log");
    let script = cleanup_script(&log);

    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/child.sock"), 42);
    entry.cleanup_environment_id = Some("env-once".to_string());
    entry.cleanup_argv = vec![script.to_string_lossy().to_string()];
    registry.lock().unwrap().insert("child".to_string(), entry);

    let a = cleanup_registered_once(&registry, "child");
    let b = cleanup_registered_once(&registry, "child");
    tokio::join!(a, b);

    let text = std::fs::read_to_string(&log).unwrap();
    assert_eq!(text.lines().collect::<Vec<_>>(), vec!["env-once"]);
    let guard = registry.lock().unwrap();
    let entry = guard.get("child").unwrap();
    assert!(entry.cleanup_environment_id.is_none());
    assert!(entry.cleanup_argv.is_empty());
}

#[test]
fn cleanup_removed_entries_runs_pid_zero_script_plan_before_discard() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("cleanup.log");
    let script = cleanup_script(&log);

    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/script.sock"), 0);
    entry.cleanup_environment_id = Some("env-kill".into());
    entry.cleanup_argv = vec![script.to_string_lossy().to_string()];
    let mut removed = vec![("child".to_string(), entry)];

    super::subagent_cleanup::cleanup_removed_entries_once(&mut removed);
    super::subagent_cleanup::cleanup_removed_entries_once(&mut removed);

    let text = std::fs::read_to_string(&log).unwrap();
    assert_eq!(text.lines().collect::<Vec<_>>(), vec!["env-kill"]);
    assert!(removed[0].1.cleanup_environment_id.is_none());
    assert!(removed[0].1.cleanup_argv.is_empty());
}

#[test]
fn teardown_all_claims_pid_zero_cleanup_before_registry_clear() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("cleanup.log");
    let script = cleanup_script(&log);

    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/script.sock"), 0);
    entry.cleanup_environment_id = Some("env-teardown".into());
    entry.cleanup_argv = vec![script.to_string_lossy().to_string()];
    registry.lock().unwrap().insert("child".to_string(), entry);

    let removed = super::spawn_registry::shutdown_all_with_count(&registry);

    assert_eq!(removed, 1);
    assert!(registry.lock().unwrap().is_empty());
    let text = std::fs::read_to_string(&log).unwrap();
    assert_eq!(text.lines().collect::<Vec<_>>(), vec!["env-teardown"]);
}

#[tokio::test]
async fn cleanup_registered_once_stops_the_committed_environment_entry() {
    use crate::domain::environment_registry::{
        EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus, mint_environment_uuid,
    };

    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("cleanup.log");
    let script = cleanup_script(&log);

    let environments = EnvironmentRegistry::new();
    let env_ref = environments.mint_ref();
    environments.commit(EnvironmentRecord {
        environment_ref: env_ref.clone(),
        environment_id: "env-exit".to_string(),
        environment_uuid: mint_environment_uuid(),
        name: None,
        workspace_path: PathBuf::from("/workspace"),
        repository: String::new(),
        script_name: "default".to_string(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec![],
        members: vec!["child".to_string()],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
    });

    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/child.sock"), 42);
    entry.cleanup_environment_id = Some("env-exit".to_string());
    entry.cleanup_argv = vec![script.to_string_lossy().to_string()];
    entry.environment_registry = Some(environments.clone());
    entry.environment_ref = Some(env_ref.clone());
    registry.lock().unwrap().insert("child".to_string(), entry);

    // The monitor EOF path (normal child exit) runs exactly this function: the
    // final member's exit claims the environment cleanup exactly once. Script
    // sets without a retained kill fall back to the rollback cleanup plan, and
    // the stopped record stays listed (#1369 slice 2: refs never reused).
    cleanup_registered_once(&registry, "child").await;
    let record = environments
        .get(&env_ref)
        .expect("stopped record stays listed");
    assert_eq!(record.status, EnvironmentStatus::Stopped);
    assert!(record.members.is_empty());
    assert_eq!(std::fs::read_to_string(&log).unwrap().trim(), "env-exit");

    // Second run is a no-op: the claim was consumed.
    cleanup_registered_once(&registry, "child").await;
    assert_eq!(std::fs::read_to_string(&log).unwrap().trim(), "env-exit");
}

#[test]
fn run_kill_sync_reports_missing_argv_and_failures_truthfully() {
    assert!(
        super::subagent_cleanup::run_kill_sync("env-x", &[])
            .unwrap_err()
            .contains("no retained kill argv")
    );
    assert!(
        super::subagent_cleanup::run_kill_sync("env-x", &["false".to_string()])
            .unwrap_err()
            .contains("retained kill exited")
    );
    assert!(
        super::subagent_cleanup::run_kill_sync("env-x", &["/definitely/not/a/kill".to_string()])
            .unwrap_err()
            .contains("failed to invoke"),
    );
    assert!(super::subagent_cleanup::run_kill_sync("env-x", &["true".to_string()]).is_ok());
}
