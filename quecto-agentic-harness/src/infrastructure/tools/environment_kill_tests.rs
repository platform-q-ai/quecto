use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentStatus, mint_environment_uuid,
};
use crate::environment_control_app::EnvironmentKillPort;

use super::ScriptEnvironmentKill;
use crate::infrastructure::tools::subagent_registry::new_registry;

fn record(retained_kill_argv: Vec<String>) -> EnvironmentRecord {
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
        members: vec![],
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
fn kill_environment_reports_successful_retained_kill() {
    let port = ScriptEnvironmentKill::new(new_registry(), None);
    let result = block_on(port.kill_environment(&record(vec!["true".into()])));

    assert!(result.is_ok(), "{result:?}");
}

#[test]
fn kill_environment_reports_retained_kill_failure() {
    let port = ScriptEnvironmentKill::new(new_registry(), None);
    let err = block_on(port.kill_environment(&record(vec!["false".into()]))).unwrap_err();

    assert!(err.contains("retained kill exited"), "{err}");
}
