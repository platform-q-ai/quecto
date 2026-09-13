//! Contract for [`EnvironmentProcessCommands`] (#1939), proven on the
//! production script adapter: each retained argv runs exactly once against
//! the environment's runtime id, a script's failure is reported in the
//! adapter's own words (never mistaken for success), the inspect result is
//! parsed strictly, and an empty argv is refused rather than executed.
use std::sync::Arc;

use quecto::application::environments::ports::EnvironmentProcessCommands;
use quecto::infrastructure::tools::environment_commands::ScriptEnvironmentCommands;

fn script(dir: &std::path::Path, name: &str, body: &str) -> Vec<String> {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/usr/bin/env bash\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    vec![path.to_string_lossy().into_owned()]
}

fn logging(dir: &std::path::Path, name: &str, log: &std::path::Path, exit: i32) -> Vec<String> {
    script(
        dir,
        name,
        &format!(
            "echo \"$QUECTO_CONTAINER_ENVIRONMENT_ID\" >> '{}'\nexit {exit}",
            log.display()
        ),
    )
}

fn port() -> Arc<dyn EnvironmentProcessCommands + Send + Sync> {
    Arc::new(ScriptEnvironmentCommands::default())
}

#[tokio::test]
async fn kill_runs_once_against_the_runtime_id_and_reports_the_script_outcome() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("kill.log");
    let ok = logging(temp.path(), "kill-ok.sh", &log, 0);
    let bad = logging(temp.path(), "kill-bad.sh", &log, 3);
    let port = port();
    port.run_retained_kill("env-contract", &ok).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(&log).unwrap().trim(),
        "env-contract"
    );

    let err = port
        .run_retained_kill("env-contract", &bad)
        .await
        .unwrap_err();
    assert!(err.contains("retained kill exited"), "{err}");
    assert_eq!(
        std::fs::read_to_string(&log).unwrap().lines().count(),
        2,
        "the failing kill still ran exactly once"
    );

    let err = port
        .run_retained_kill("env-contract", &["/definitely/not/a/kill".into()])
        .await
        .unwrap_err();
    assert!(err.contains("failed to invoke"), "{err}");
    let err = port
        .run_retained_kill("env-contract", &[])
        .await
        .unwrap_err();
    assert!(err.contains("no retained kill argv"), "{err}");
}

#[tokio::test]
async fn cleanup_is_best_effort_and_never_runs_an_empty_argv() {
    let temp = tempfile::tempdir().unwrap();
    let log = temp.path().join("cleanup.log");
    let port = port();
    port.run_retained_cleanup("env-contract", &logging(temp.path(), "c.sh", &log, 1))
        .await;
    assert_eq!(
        std::fs::read_to_string(&log).unwrap().trim(),
        "env-contract"
    );
    port.run_retained_cleanup("env-contract", &[]).await;
    port.run_retained_cleanup("env-contract", &["/definitely/not/a/cleanup".into()])
        .await;
}

#[tokio::test]
async fn inspect_parses_the_strict_wire_and_reports_failures_and_timeouts() {
    let temp = tempfile::tempdir().unwrap();
    let port = port();
    let good = script(
        temp.path(),
        "inspect.sh",
        r#"printf '{"status":"exited","metadata":{"exit_code":7,"env":"%s"}}' "$QUECTO_CONTAINER_ENVIRONMENT_ID""#,
    );
    let metadata = port
        .run_retained_inspect("env-contract", &good)
        .await
        .unwrap();
    assert_eq!(metadata["exit_code"], 7);
    assert_eq!(metadata["env"], "env-contract");
    assert_eq!(metadata["inspect_status"], "exited");

    let junk = script(
        temp.path(),
        "junk.sh",
        r#"printf '{"metadata":{},"extra":1}'"#,
    );
    let err = port
        .run_retained_inspect("env-contract", &junk)
        .await
        .unwrap_err();
    assert!(err.contains("inspect"), "{err}");

    let failing = script(temp.path(), "fail.sh", "echo boom >&2; exit 2");
    let err = port
        .run_retained_inspect("env-contract", &failing)
        .await
        .unwrap_err();
    assert!(
        err.contains("retained inspect exited") && err.contains("boom"),
        "{err}"
    );

    let err = port
        .run_retained_inspect("env-contract", &[])
        .await
        .unwrap_err();
    assert!(err.contains("no retained inspect argv"), "{err}");
}

/// The port is usable off a runtime too (the final-member path runs on a
/// blocking worker under a plain executor).
#[test]
fn port_answers_without_an_async_runtime() {
    let port = port();
    assert!(futures::executor::block_on(port.run_retained_kill("env-x", &["true".into()])).is_ok());
    assert!(
        futures::executor::block_on(port.run_retained_kill("env-x", &["false".into()])).is_err()
    );
}
