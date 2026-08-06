//! Contract suite for [`EnvironmentKillPort`] (#1369 slice 2): the real
//! script-managed adapter must terminate every member agent, run the
//! environment's retained kill argv exactly once against the runtime
//! environment id, and report failures truthfully so the use case can persist
//! a retryable cleanup-failed state.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use quecto::application::environment_control::EnvironmentKillPort;
use quecto::domain::environment_registry::{
    EnvironmentRecord, EnvironmentStatus, mint_environment_uuid,
};
use quecto::infrastructure::tools::agent_cmd::{SubagentEntry, SubagentRegistry};
use quecto::infrastructure::tools::environment_kill::ScriptEnvironmentKill;

fn kill_script(dir: &std::path::Path, log: &std::path::Path, exit_code: i32) -> PathBuf {
    let script = dir.join(format!("kill-{exit_code}.sh"));
    std::fs::write(
        &script,
        format!(
            "#!/usr/bin/env bash\necho \"$QUECTO_CONTAINER_ENVIRONMENT_ID\" >> '{}'\nexit {exit_code}\n",
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

fn record(kill_argv: Vec<String>, members: Vec<String>) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: "C1".into(),
        environment_id: "env-contract".into(),
        environment_uuid: mint_environment_uuid(),
        name: None,
        workspace_path: PathBuf::from("/workspace"),
        repository: String::new(),
        script_name: "default".into(),
        retained_exec_argv: vec![],
        retained_kill_argv: kill_argv,
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members,
        status: EnvironmentStatus::Killing,
        metadata: serde_json::json!({}),
        last_error: None,
    }
}

#[tokio::test]
async fn script_kill_port_terminates_members_and_runs_retained_kill_once() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("kill.log");
    let script = kill_script(temp.path(), &log, 0);

    let subagents: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let entry = SubagentEntry::new(PathBuf::from("/tmp/member.sock"), 0);
    subagents
        .lock()
        .unwrap()
        .insert("member-uuid".to_string(), entry);

    let port = ScriptEnvironmentKill::new(subagents.clone(), None);
    let rec = record(
        vec![script.to_string_lossy().to_string()],
        vec!["member-uuid".to_string()],
    );
    port.kill_environment(&rec).await.unwrap();

    // Member entry is gone from the subagent registry.
    assert!(subagents.lock().unwrap().is_empty());
    // The retained kill ran exactly once against the runtime environment id.
    let logged = std::fs::read_to_string(&log).unwrap();
    assert_eq!(logged.lines().collect::<Vec<_>>(), vec!["env-contract"]);
}

#[tokio::test]
async fn script_kill_port_reports_kill_failure_truthfully() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("kill.log");
    let script = kill_script(temp.path(), &log, 3);

    let subagents: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let port = ScriptEnvironmentKill::new(subagents, None);
    let rec = record(vec![script.to_string_lossy().to_string()], vec![]);
    let err = port.kill_environment(&rec).await.unwrap_err();
    assert!(err.contains("retained kill"), "{err}");
    // The kill was still attempted (evidence in the log) before failing.
    assert_eq!(
        std::fs::read_to_string(&log).unwrap().trim(),
        "env-contract"
    );
}

#[tokio::test]
async fn script_kill_port_refuses_environments_without_a_retained_kill() {
    let subagents: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let port = ScriptEnvironmentKill::new(subagents, None);
    let err = port
        .kill_environment(&record(vec![], vec![]))
        .await
        .unwrap_err();
    assert!(err.contains("no retained kill argv"), "{err}");
}

/// #1390 review finding: the no-kill-argv refusal must happen BEFORE member
/// termination — otherwise members die while the environment is stuck in a
/// deterministically unrecoverable cleanup-failed state.
#[tokio::test]
async fn script_kill_port_refusal_leaves_members_untouched() {
    let subagents: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let entry = SubagentEntry::new(PathBuf::from("/tmp/member.sock"), 0);
    subagents
        .lock()
        .unwrap()
        .insert("member-uuid".to_string(), entry);
    let port = ScriptEnvironmentKill::new(subagents.clone(), None);
    let err = port
        .kill_environment(&record(vec![], vec!["member-uuid".to_string()]))
        .await
        .unwrap_err();
    assert!(err.contains("no retained kill argv"), "{err}");
    assert!(
        subagents.lock().unwrap().contains_key("member-uuid"),
        "members must survive a refused kill_container"
    );
}
