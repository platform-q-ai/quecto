use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentStatus, mint_environment_uuid,
};
use crate::environment_control_app::EnvironmentKillPort;

use super::ScriptEnvironmentKill;
use crate::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentStatus, new_registry,
};

fn record(retained_kill_argv: Vec<String>) -> EnvironmentRecord {
    record_with_members(retained_kill_argv, vec![])
}

fn record_with_members(retained_kill_argv: Vec<String>, members: Vec<String>) -> EnvironmentRecord {
    EnvironmentRecord {
        environment_ref: "C1".into(),
        environment_id: "env-kill-test".into(),
        environment_uuid: mint_environment_uuid(),
        name: Some("kill-test".into()),
        workspace_path: std::path::PathBuf::from("/tmp/kill-test"),
        repository: String::new(),
        script_name: "default".into(),
        retained_exec_argv: vec![],
        retained_kill_argv,
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members,
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
    }
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}

#[test]
fn kill_environment_rejects_missing_retained_kill_argv_before_running_processes() {
    let port = ScriptEnvironmentKill::new(new_registry(), None);
    let err = block_on(port.kill_environment(&record(vec![]))).unwrap_err();

    assert!(err.contains("has no retained kill argv"), "{err}");
    assert!(err.contains("kill_container"), "{err}");
}

#[test]
fn kill_environment_terminates_members_broadcasts_state_and_runs_retained_kill() {
    let registry = new_registry();
    let mut parent = SubagentEntry::new(std::path::PathBuf::from("/tmp/parent.sock"), 0);
    parent.cleanup_environment_id = Some("env-kill-test".into());
    let (parent_exit_tx, mut parent_exit_rx) = tokio::sync::watch::channel(None);
    parent.exit_signal_tx = Some(parent_exit_tx);
    let mut child = SubagentEntry::new(std::path::PathBuf::from("/tmp/child.sock"), 0);
    let (child_exit_tx, _child_exit_rx) = tokio::sync::watch::channel(None);
    child.exit_signal_tx = Some(child_exit_tx);
    registry.lock().unwrap().insert("parent".into(), parent);
    registry.lock().unwrap().insert("child".into(), child);
    let (broadcast_tx, mut broadcast_rx) = tokio::sync::broadcast::channel(4);
    let port = ScriptEnvironmentKill::new(registry.clone(), Some(broadcast_tx));

    let result = block_on(port.kill_environment(&record_with_members(
        vec!["true".into()],
        vec!["parent".into()],
    )));

    assert!(result.is_ok(), "{result:?}");
    assert_eq!(
        registry.lock().unwrap()["parent"].status,
        SubagentStatus::Exited
    );
    assert!(registry.lock().unwrap().contains_key("child"));
    assert_eq!(
        parent_exit_rx.borrow_and_update().as_ref().unwrap().signal,
        Some(15)
    );
    let event = broadcast_rx.try_recv().unwrap();
    assert!(event.contains("state_changed"), "{event}");
}

#[test]
fn kill_environment_reports_retained_kill_failure() {
    let port = ScriptEnvironmentKill::new(new_registry(), None);
    let err = block_on(port.kill_environment(&record(vec!["false".into()]))).unwrap_err();

    assert!(err.contains("retained kill exited"), "{err}");
}
