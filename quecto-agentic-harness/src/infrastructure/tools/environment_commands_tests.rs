//! Script adapters of the environments capability's ports (#1939).
use crate::application::environments::ports::EnvironmentProcessCommands;

#[test]
fn run_kill_sync_reports_missing_argv_and_failures_truthfully() {
    assert!(
        super::run_kill_sync("env-x", &[])
            .unwrap_err()
            .contains("no retained kill argv")
    );
    assert!(
        super::run_kill_sync("env-x", &["false".to_string()])
            .unwrap_err()
            .contains("retained kill exited")
    );
    assert!(
        super::run_kill_sync("env-x", &["/definitely/not/a/kill".to_string()])
            .unwrap_err()
            .contains("failed to invoke"),
    );
    assert!(super::run_kill_sync("env-x", &["true".to_string()]).is_ok());
}

/// #1391 review: the inspect subprocess is bounded — a hung script is killed
/// and reported as a timeout instead of stalling the death pipeline. The
/// bound is the shared script runner's (#2024 S4b); this pins the inspect
/// path's own wording, which names the retry.
#[test]
fn inspect_kills_hung_scripts_and_names_the_retry() {
    let started = std::time::Instant::now();
    let err = super::run_inspect_sync_bounded(
        "env-x",
        &["sleep".to_string(), "30".to_string()],
        std::time::Duration::from_millis(200),
    )
    .unwrap_err();
    assert!(err.contains("timed out"), "{err}");
    assert!(err.contains("retained argv kept for retry"), "{err}");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert!(super::INSPECT_TIMEOUT >= std::time::Duration::from_secs(1));
}

/// The kill adapter answers the port truthfully whether or not a runtime
/// is present: success only when the script reports it.
#[tokio::test]
async fn script_kill_adapter_reports_the_script_outcome_on_a_runtime() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("kill.log");
    let script = temp.path().join("kill.sh");
    std::fs::write(
        &script,
        format!(
            "#!/usr/bin/env bash\necho \"$QUECTO_CONTAINER_ENVIRONMENT_ID\" >> '{}'\nexit ${{KILL_EXIT:-0}}\n",
            log.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let argv = vec!["bash".to_string(), script.to_string_lossy().to_string()];
    super::ScriptEnvironmentCommands::default()
        .run_retained_kill("env-runtime", &argv)
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(&log).unwrap().trim(), "env-runtime");
    let err = super::ScriptEnvironmentCommands::default()
        .run_retained_kill("env-runtime", &[])
        .await
        .unwrap_err();
    assert!(err.contains("no retained kill argv"), "{err}");
    super::ScriptEnvironmentCommands::default()
        .run_retained_cleanup("env-runtime", &argv)
        .await;
    assert_eq!(
        std::fs::read_to_string(&log).unwrap().lines().count(),
        2,
        "cleanup ran the same script once more"
    );
    super::ScriptEnvironmentCommands::default()
        .run_retained_cleanup("env-runtime", &[])
        .await;
}

/// Without a runtime the adapter runs the script inline (the final-member
/// path runs on a blocking worker under `block_on`).
#[test]
fn script_adapters_run_inline_without_a_runtime() {
    let ok = futures::executor::block_on(
        super::ScriptEnvironmentCommands::default()
            .run_retained_kill("env-x", &["true".to_string()]),
    );
    assert!(ok.is_ok());
    let err = futures::executor::block_on(
        super::ScriptEnvironmentCommands::default().run_retained_inspect("env-x", &[]),
    )
    .unwrap_err();
    assert!(err.contains("no retained inspect argv"), "{err}");
}
