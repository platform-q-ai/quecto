use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use super::subagent_cleanup::{
    FinalizeMode, cleanup_registered_once, cleanup_removed_entries_once, output_with_timeout,
};
use super::subagent_registry::{SubagentEntry, SubagentRegistry};

fn cleanup_script(log: &std::path::Path) -> std::path::PathBuf {
    let script = log.parent().unwrap().join("cleanup.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho \"$QUECTO_CONTAINER_ENVIRONMENT_ID\" >> '{}'\n",
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

    super::subagent_cleanup::cleanup_removed_entries_sync(&mut removed);
    super::subagent_cleanup::cleanup_removed_entries_sync(&mut removed);

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
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
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

fn committed_env_record(
    env_ref: &str,
    kill_argv: Vec<String>,
    cleanup_argv: Vec<String>,
    members: Vec<String>,
) -> crate::domain::environment_registry::EnvironmentRecord {
    use crate::domain::environment_registry::{EnvironmentRecord, mint_environment_uuid};
    EnvironmentRecord {
        environment_ref: env_ref.to_string(),
        environment_id: format!("runtime-{env_ref}"),
        environment_uuid: mint_environment_uuid(),
        name: None,
        workspace_path: PathBuf::from("/workspace"),
        repository: String::new(),
        script_name: "default".to_string(),
        retained_exec_argv: vec![],
        retained_kill_argv: kill_argv,
        retained_cleanup_argv: cleanup_argv,
        retained_inspect_argv: vec![],
        members,
        status: crate::domain::environment_registry::EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
    }
}

/// Finding fix (#1390 review): the final member may be a joiner whose entry
/// never carried a cleanup plan. The environment record's retained cleanup
/// argv must cover the fallback even after the creator exited first.
#[tokio::test]
async fn final_joiner_exit_falls_back_to_the_record_retained_cleanup() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("cleanup.log");
    let script = cleanup_script(&log);

    let environments = crate::domain::environment_registry::EnvironmentRegistry::new();
    let env_ref = environments.mint_ref();
    environments.commit(committed_env_record(
        &env_ref,
        vec![],
        vec!["/bin/sh".to_string(), script.to_string_lossy().to_string()],
        vec!["creator".to_string(), "joiner".to_string()],
    ));

    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    // The creator's entry holds the only per-entry cleanup plan.
    let mut creator = SubagentEntry::new(PathBuf::from("/tmp/creator.sock"), 0);
    creator.cleanup_environment_id = Some("plan-env".to_string());
    creator.cleanup_argv = vec!["true".to_string()];
    creator.environment_registry = Some(environments.clone());
    creator.environment_ref = Some(env_ref.clone());
    registry
        .lock()
        .unwrap()
        .insert("creator".to_string(), creator);
    let mut joiner = SubagentEntry::new(PathBuf::from("/tmp/joiner.sock"), 0);
    joiner.environment_registry = Some(environments.clone());
    joiner.environment_ref = Some(env_ref.clone());
    registry
        .lock()
        .unwrap()
        .insert("joiner".to_string(), joiner);

    // Creator exits first (non-final): no teardown yet.
    cleanup_registered_once(&registry, "creator").await;
    assert!(!log.exists(), "non-final exit must not tear down");
    // Joiner exits last: the record's retained cleanup runs exactly once.
    cleanup_registered_once(&registry, "joiner").await;
    let record = environments.get(&env_ref).unwrap();
    assert_eq!(
        record.status,
        crate::domain::environment_registry::EnvironmentStatus::Stopped
    );
    assert_eq!(
        std::fs::read_to_string(&log).unwrap().trim(),
        format!("runtime-{env_ref}")
    );
}

/// Finding fix (#1390 review): a launch rollback after creation runs the
/// retained `cleanup`, never the retained `kill`, matching the documented
/// script contract for initial-prompt failures.
#[tokio::test]
async fn launch_rollback_runs_retained_cleanup_instead_of_kill() {
    let temp = tempfile::tempdir().unwrap();
    let cleanup_log = temp.path().join("cleanup.log");
    let cleanup = cleanup_script(&cleanup_log);
    let kill_log = temp.path().join("kill.log");
    let kill = {
        let script = temp.path().join("kill.sh");
        std::fs::write(
            &script,
            format!(
                "#!/usr/bin/env bash\necho killed >> '{}'\n",
                kill_log.display()
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
    };

    let environments = crate::domain::environment_registry::EnvironmentRegistry::new();
    let env_ref = environments.mint_ref();
    environments.commit(committed_env_record(
        &env_ref,
        vec!["/bin/sh".to_string(), kill.to_string_lossy().to_string()],
        vec!["/bin/sh".to_string(), cleanup.to_string_lossy().to_string()],
        vec!["creator".to_string()],
    ));

    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/creator.sock"), 0);
    entry.environment_registry = Some(environments.clone());
    entry.environment_ref = Some(env_ref.clone());
    let mut removed = vec![("creator".to_string(), entry)];
    super::subagent_cleanup::cleanup_removed_entries_once(
        &mut removed,
        super::subagent_cleanup::FinalizeMode::LaunchRollback,
    )
    .await;

    assert!(cleanup_log.exists(), "rollback must run retained cleanup");
    assert!(!kill_log.exists(), "rollback must not run retained kill");
    assert_eq!(
        environments.get(&env_ref).unwrap().status,
        crate::domain::environment_registry::EnvironmentStatus::Stopped
    );
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

#[tokio::test]
async fn owned_launch_rollback_discards_the_environment_record_entirely() {
    let temp = tempfile::tempdir().unwrap();
    let cleanup_log = temp.path().join("cleanup.log");
    let cleanup = cleanup_script(&cleanup_log);

    let environments = crate::domain::environment_registry::EnvironmentRegistry::new();
    let env_ref = environments.mint_ref();
    environments.commit(committed_env_record(
        &env_ref,
        vec![],
        vec!["/bin/sh".to_string(), cleanup.to_string_lossy().to_string()],
        vec!["creator".to_string()],
    ));

    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/creator.sock"), 0);
    entry.environment_registry = Some(environments.clone());
    entry.environment_ref = Some(env_ref.clone());
    let mut removed = vec![("creator".to_string(), entry)];
    super::subagent_cleanup::cleanup_removed_entries_once(
        &mut removed,
        super::subagent_cleanup::FinalizeMode::LaunchRollbackOwned,
    )
    .await;

    assert!(cleanup_log.exists(), "rollback must run retained cleanup");
    // A creator rollback leaves NO record: the environment never became
    // usable, matching pre-registration rollback. The ref stays consumed.
    assert!(environments.get(&env_ref).is_none());
    assert!(environments.entries().is_empty());
    assert_eq!(environments.mint_ref(), "C2");
}

fn logging_fail_script(log: &std::path::Path, dir: &std::path::Path) -> String {
    let script = dir.join("inspect-fail.sh");
    std::fs::write(
        &script,
        format!(
            "#!/usr/bin/env bash\necho \"$QUECTO_CONTAINER_ENVIRONMENT_ID\" >> '{}'\nexit 1\n",
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
    script.to_string_lossy().to_string()
}

/// #1391 review: a parent-initiated kill is not a post-mortem — members it
/// terminates are never inspected, so a failing inspect script cannot stick
/// a permanent error onto a cleanly killed environment.
#[tokio::test]
async fn parent_kill_members_are_not_inspected_and_no_error_sticks() {
    use crate::domain::environment_registry::{EnvironmentRegistry, EnvironmentStatus};

    let temp = tempfile::tempdir().unwrap();
    let inspect_log = temp.path().join("inspect.log");
    let inspect = logging_fail_script(&inspect_log, temp.path());

    let environments = EnvironmentRegistry::new();
    let env_ref = environments.mint_ref();
    let mut record = committed_env_record(
        &env_ref,
        vec!["true".into()],
        vec![],
        vec!["member".to_string()],
    );
    record.retained_inspect_argv = vec![inspect];
    environments.commit(record);

    // The kill_container flow claims the environment first, then tears down
    // members with FinalizeMode::ParentKill.
    let claim = environments.begin_kill(&env_ref).unwrap();
    let mut entry = SubagentEntry::new(PathBuf::from("/tmp/member.sock"), 0);
    entry.environment_registry = Some(environments.clone());
    entry.environment_ref = Some(env_ref.clone());
    let mut removed = vec![("member".to_string(), entry)];
    cleanup_removed_entries_once(&mut removed, FinalizeMode::ParentKill).await;
    environments.complete_kill(claim);

    assert!(
        !inspect_log.exists(),
        "parent-kill member must not be inspected"
    );
    let record = environments.get(&env_ref).unwrap();
    assert_eq!(record.status, EnvironmentStatus::Stopped);
    assert!(record.members.is_empty());
    assert!(record.last_error.is_none());
}

/// #1391 review: the inspect subprocess is bounded — a hung script is killed
/// and reported as a timeout instead of stalling the death pipeline.
#[test]
fn output_with_timeout_kills_hung_commands_and_passes_fast_ones() {
    let mut hung = std::process::Command::new("sleep");
    hung.arg("30");
    let started = std::time::Instant::now();
    let err = output_with_timeout(hung, std::time::Duration::from_millis(200)).unwrap_err();
    assert!(err.contains("timed out"), "{err}");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));

    let mut fast = std::process::Command::new("echo");
    fast.arg("ok");
    fast.stdout(std::process::Stdio::piped());
    fast.stderr(std::process::Stdio::piped());
    let output = output_with_timeout(fast, std::time::Duration::from_secs(5)).unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "ok");
}
