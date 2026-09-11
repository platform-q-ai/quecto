use super::*;

#[tokio::test]
async fn trial_ordinary_commands_do_not_inherit_swarm_launch_identity() {
    let env = HashMap::from([
        ("QUECTO_SWARM_CHECKOUT".into(), "/live/checkout".into()),
        ("QUECTO_SWARM_MEMBER".into(), "coordinator".into()),
        ("QUECTO_SWARM_RESERVATION".into(), "live-token".into()),
        ("QUECTO_SWARM_BOOTSTRAP".into(), "1".into()),
        ("QUECTO_SWARM_CONTAINER".into(), "isolated-pid-v1".into()),
        ("QUECTO_SWARM_HOST_PID_NS".into(), "pid:[1]".into()),
        ("RUST_LOG".into(), "info".into()),
        ("TRIAL_SENTINEL".into(), "preserved".into()),
    ]);
    let output = super::build_shell_command(&PathBuf::from("/tmp"), "env", Some(&env))
        .output()
        .await
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("TRIAL_SENTINEL=preserved"));
    assert!(
        !stdout.contains("QUECTO_SWARM_"),
        "test runtimes inherit live enrollment: {stdout}"
    );
    assert!(
        !stdout.lines().any(|line| line.starts_with("RUST_LOG=")),
        "tool children must not inherit the harness log level (#1924): {stdout}"
    );
}
