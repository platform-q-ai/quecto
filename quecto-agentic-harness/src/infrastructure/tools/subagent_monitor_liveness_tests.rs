//! Slice 3 (#1369) behavioral liveness tests through the production monitor
//! seam: repeated EOF/reset death signals trigger ONE inspect and ONE
//! terminal transition, and a connection reset classifies as the same pushed
//! death signal as a clean EOF.

use super::*;
use crate::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus, mint_environment_uuid,
};
use crate::infrastructure::tools::subagent_registry::ExitSignalKind;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn write_inspect_script(dir: &std::path::Path, log: &std::path::Path) -> std::path::PathBuf {
    write_inspect_script_with_body(
        dir,
        &format!(
            "echo \"inspect ${{QUECTO_CONTAINER_ENVIRONMENT_ID:-}}\" >> '{}'\nprintf '{{\"status\":\"dead\",\"metadata\":{{\"cause\":\"oom-killed\"}}}}'\n",
            log.display()
        ),
    )
}

/// A logging inspect script that FAILS (exit 1) so failure-path tests can
/// observe the invocation count instead of relying on unobservable side
/// effects.
fn write_failing_inspect_script(
    dir: &std::path::Path,
    log: &std::path::Path,
) -> std::path::PathBuf {
    write_inspect_script_with_body(
        dir,
        &format!("echo \"inspect-failed\" >> '{}'\nexit 1\n", log.display()),
    )
}

fn write_inspect_script_with_body(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
    let script = dir.join("inspect.sh");
    std::fs::write(&script, format!("#!/usr/bin/env bash\n{body}")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    script
}

fn environment_with_inspect(inspect: Vec<String>) -> (EnvironmentRegistry, String) {
    let environments = EnvironmentRegistry::new();
    let env_ref = environments.mint_ref();
    environments.commit(EnvironmentRecord {
        environment_ref: env_ref.clone(),
        environment_id: "env-live".into(),
        environment_uuid: mint_environment_uuid(),
        name: None,
        workspace_path: std::path::PathBuf::from("/tmp/ws"),
        repository: String::new(),
        script_name: "default".into(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec![],
        retained_cleanup_argv: vec![],
        retained_inspect_argv: inspect,
        members: vec![],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
    });
    (environments, env_ref)
}

#[tokio::test]
async fn repeated_death_signals_trigger_one_inspect_and_one_terminal_transition() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("inspect-log.txt");
    let script = write_inspect_script(dir.path(), &log);
    let (environments, env_ref) = environment_with_inspect(vec![script.display().to_string()]);
    environments.add_member(&env_ref, "agent-1").unwrap();

    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let mut entry = SubagentEntry::new(dir.path().join("agent-1.sock"), 0);
    entry.environment_registry = Some(environments.clone());
    entry.environment_ref = Some(env_ref.clone());
    let (exit_tx, mut exit_rx) = super::super::subagent_registry::new_exit_signal_channel();
    entry.exit_signal_tx = Some(exit_tx);
    registry
        .lock()
        .unwrap()
        .insert("agent-1".to_string(), entry);

    // The production death-signal entry point (monitor EOF *and* reset both
    // route here) delivered twice for the same member.
    notify_child_exited(
        &registry,
        "agent-1",
        None,
        None,
        ExitSignalKind::ConnectionClosed,
    )
    .await;
    notify_child_exited(
        &registry,
        "agent-1",
        None,
        None,
        ExitSignalKind::ConnectionClosed,
    )
    .await;

    let text = std::fs::read_to_string(&log).unwrap_or_default();
    assert_eq!(
        text.lines().count(),
        1,
        "repeated EOF/reset must invoke the retained inspect exactly once: {text:?}"
    );
    let record = environments.get(&env_ref).unwrap();
    assert_eq!(record.metadata["cause"], serde_json::json!("oom-killed"));
    assert!(record.members.is_empty());
    assert!(
        !registry.lock().unwrap().contains_key("agent-1"),
        "dead script-managed member is cascade-pruned"
    );
    // The pushed death fed the existing exit signal so lifecycle observers wake.
    assert!(exit_rx.borrow_and_update().is_some());
}

#[tokio::test]
async fn duplicate_signals_after_inspect_failure_still_inspect_once() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("inspect-fail-log.txt");
    let script = write_failing_inspect_script(dir.path(), &log);
    let (environments, env_ref) = environment_with_inspect(vec![script.display().to_string()]);
    environments.add_member(&env_ref, "agent-2").unwrap();
    let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
    let mut entry = SubagentEntry::new(dir.path().join("agent-2.sock"), 0);
    entry.environment_registry = Some(environments.clone());
    entry.environment_ref = Some(env_ref.clone());
    registry
        .lock()
        .unwrap()
        .insert("agent-2".to_string(), entry);

    notify_child_exited(
        &registry,
        "agent-2",
        None,
        None,
        ExitSignalKind::ConnectionClosed,
    )
    .await;
    notify_child_exited(
        &registry,
        "agent-2",
        None,
        None,
        ExitSignalKind::ConnectionClosed,
    )
    .await;

    let text = std::fs::read_to_string(&log).unwrap_or_default();
    assert_eq!(
        text.lines().count(),
        1,
        "repeated signals after an inspect failure must not re-run it: {text:?}"
    );
    let record = environments.get(&env_ref).unwrap();
    let last_error = record.last_error.clone().unwrap_or_default();
    assert!(last_error.contains("inspect"), "last_error: {last_error}");
    assert_eq!(
        record.retained_inspect_argv,
        vec![script.display().to_string()],
        "retained context survives the failure"
    );
}

/// A connection RESET (not just clean EOF) is classified as the pushed death
/// signal: the monitor read path reports `Closed`, the same terminal path a
/// clean EOF takes.
#[tokio::test]
async fn connection_reset_classifies_as_pushed_death_signal() {
    struct ResetReader;
    impl tokio::io::AsyncRead for ResetReader {
        fn poll_read(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            std::task::Poll::Ready(Err(std::io::Error::from(
                std::io::ErrorKind::ConnectionReset,
            )))
        }
    }
    let mut reader = tokio::io::BufReader::new(ResetReader);
    let mut buf = Vec::new();
    let read = read_monitor_message(&mut reader, &mut buf, "agent-reset").await;
    assert!(matches!(read, MonitorRead::Closed));
}
