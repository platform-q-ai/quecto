//! The official Docker adapter's `kill.sh` against a fake runtime CLI
//! (#2024 S4d, round 2 F-C of #2033): what it asks the runtime to remove.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use quecto::application::environments::ports::{
    EnvironmentMemberShutdown, MemberShutdownReport, PortFuture,
};
use quecto::application::environments::use_cases::{StopEnvironment, StopEnvironmentError};
use quecto::domain::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus, EnvironmentTarget,
};
use quecto::infrastructure::tools::environment_commands::ScriptEnvironmentCommands;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

/// A `podman` on a controlled PATH that records every argv line it is
/// called with and succeeds.
fn fake_podman(bin: &Path, log: &Path) {
    fake_podman_with_behavior(bin, log, "exit 0", "exit 0");
}

/// A controlled `podman` whose direct inspect and affirmative inventory can
/// fail independently. The adapter must never turn an unknown runtime failure
/// into a successful preservation receipt.
fn fake_podman_with_behavior(bin: &Path, log: &Path, inspect: &str, inventory: &str) {
    fs::create_dir_all(bin).unwrap();
    let script = bin.join("podman");
    fs::write(
        &script,
        format!(
            "#!/usr/bin/env bash\nprintf '%s\\n' \"$*\" >>'{}'\n\ncase \"${{1:-}}\" in\n  inspect) {inspect} ;;\n  ps) {inventory} ;;\n  rm) exit 0 ;;\nesac\n",
            log.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn run_kill(bin: &Path, state_dir: &Path, id: &str, op: &str) -> std::process::Output {
    Command::new(repo_root().join("scripts/container-runtime/docker/kill.sh"))
        .args(["--state-dir"])
        .arg(state_dir)
        .args(["--op", op])
        // The fake CLI first; the rest of the PATH stays for bash/coreutils.
        .env(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("QUECTO_CONTAINER_CLI", "podman")
        .env("QUECTO_CONTAINER_ENVIRONMENT_ID", id)
        .output()
        .expect("run kill.sh")
}

fn calls(log: &Path) -> Vec<String> {
    fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

struct NoMembers;

impl EnvironmentMemberShutdown for NoMembers {
    fn shutdown_members<'a>(
        &'a self,
        members: &'a [String],
    ) -> PortFuture<'a, MemberShutdownReport> {
        assert!(members.is_empty());
        Box::pin(async { MemberShutdownReport::default() })
    }
}

fn retained_registry(state_dir: &Path, runtime: &Path, id: &str) -> EnvironmentRegistry {
    let registry = EnvironmentRegistry::new();
    registry.commit(EnvironmentRecord {
        environment_ref: "C1".into(),
        environment_id: id.into(),
        environment_uuid: "uuid-stop-regression".into(),
        name: None,
        workspace_path: state_dir.join(id).join("workspace"),
        repository: "repo".into(),
        script_name: "docker".into(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec![
            runtime.to_string_lossy().into_owned(),
            "--state-dir".into(),
            state_dir.to_string_lossy().into_owned(),
            "--op".into(),
            "kill".into(),
        ],
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members: vec![],
        status: EnvironmentStatus::Retained,
        metadata: serde_json::json!({}),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: "test".into(),
        created_at: None,
    });
    registry
}

/// A state directory whose `container` file is empty (or missing: the
/// create was interrupted between the directory and the record of the
/// container it ran) still names a container the create would have called
/// `quecto-<environment_id>`; the kill removes it with the directory
/// instead of leaving an exited container behind.
#[test]
fn a_directory_without_a_recorded_container_still_removes_the_named_container() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    let log = temp.path().join("podman.log");
    fake_podman(&bin, &log);
    let state_dir = temp.path().join("state");
    for (id, container_file) in [("env-empty", Some("")), ("env-missing", None)] {
        let env_dir = state_dir.join(id);
        fs::create_dir_all(&env_dir).unwrap();
        if let Some(content) = container_file {
            fs::write(env_dir.join("container"), content).unwrap();
        }
        let output = run_kill(&bin, &state_dir, id, "kill");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!env_dir.exists(), "the state directory is removed");
    }
    assert_eq!(
        calls(&log),
        [
            "rm -f --time 1 quecto-env-empty",
            "rm -f --time 1 quecto-env-missing"
        ]
    );
    let kill_log = fs::read_to_string(state_dir.join("kill.log")).unwrap();
    assert_eq!(kill_log, "kill env-empty\nkill env-missing\n");
}

/// The recorded container name wins when there is one; a directory already
/// gone still removes the container the create would have named.
#[test]
fn a_preserving_stop_is_idempotent_and_allows_a_later_explicit_kill() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    let log = temp.path().join("podman.log");
    fake_podman(&bin, &log);
    let state_dir = temp.path().join("state");
    let env_dir = state_dir.join("env-preserved");
    fs::create_dir_all(&env_dir).unwrap();
    fs::write(env_dir.join("container"), "kept-container\n").unwrap();

    for _ in 0..2 {
        let output = run_kill(&bin, &state_dir, "env-preserved", "stop");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            env_dir.exists(),
            "preserving stop must retain workspace state"
        );
        assert_eq!(
            fs::read_to_string(env_dir.join("container")).unwrap(),
            "kept-container\n"
        );
        assert_eq!(
            fs::read_to_string(env_dir.join("runtime-stopped")).unwrap(),
            "stopped\n"
        );
    }

    assert!(
        run_kill(&bin, &state_dir, "env-preserved", "kill")
            .status
            .success()
    );
    assert!(
        !env_dir.exists(),
        "later explicit kill removes preserved state"
    );
    assert_eq!(
        calls(&log),
        [
            "inspect kept-container",
            "rm -f --time 1 kept-container",
            "inspect kept-container",
            "rm -f --time 1 kept-container",
            "rm -f --time 1 kept-container",
        ]
    );
}

#[test]
fn preserving_stop_accepts_only_affirmatively_absent_container() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    let log = temp.path().join("podman.log");
    fake_podman_with_behavior(
        &bin,
        &log,
        "printf 'no such container\\n' >&2; exit 1",
        "exit 0",
    );
    let state_dir = temp.path().join("state");
    let env_dir = state_dir.join("env-absent");
    fs::create_dir_all(&env_dir).unwrap();
    fs::write(env_dir.join("container"), "absent-container\n").unwrap();

    let output = run_kill(&bin, &state_dir, "env-absent", "stop");

    assert!(
        output.status.success(),
        "an empty successful inventory proves absence: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(env_dir.join("runtime-stopped")).unwrap(),
        "stopped\n"
    );
    assert_eq!(
        calls(&log),
        [
            "inspect absent-container",
            "ps -a --filter name=^absent-container$ --format {{.Names}}",
        ]
    );
}

#[test]
fn preserving_stop_rejects_unknown_inspect_failure_without_receipt_or_preserved_state() {
    for (id, inventory) in [
        (
            "env-daemon-failure",
            "printf 'daemon unavailable\\n' >&2; exit 125",
        ),
        (
            "env-permission-failure",
            "printf 'permission denied\\n' >&2; exit 1",
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("bin");
        let log = temp.path().join("podman.log");
        fake_podman_with_behavior(
            &bin,
            &log,
            "printf 'inspect failed\\n' >&2; exit 1",
            inventory,
        );
        let state_dir = temp.path().join("state");
        let env_dir = state_dir.join(id);
        fs::create_dir_all(&env_dir).unwrap();
        fs::write(env_dir.join("container"), "uncertain-container\n").unwrap();
        let runtime = temp.path().join("invoke-kill.sh");
        let invocation = temp.path().join("kill-invocation.log");
        fs::write(
            &runtime,
            format!(
                "#!/usr/bin/env bash\nprintf '%s\\n' \"$*\" >'{}'\nexport PATH='{}':\"$PATH\"\nexport QUECTO_CONTAINER_CLI=podman\nexec '{}' \"$@\"\n",
                invocation.display(),
                bin.display(),
                repo_root()
                    .join("scripts/container-runtime/docker/kill.sh")
                    .display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&runtime, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let registry = retained_registry(&state_dir, &runtime, id);
        let use_case = StopEnvironment::new(
            registry.clone(),
            Arc::new(NoMembers),
            Arc::new(ScriptEnvironmentCommands::default()),
        );

        // The application use case invokes the retained real adapter and must
        // keep the Retained state when that adapter cannot prove absence.
        let result = futures::executor::block_on(
            use_case.stop_container(&EnvironmentTarget::Ref("C1".into())),
        );

        assert!(
            matches!(result, Err(StopEnvironmentError::StopFailed { .. })),
            "an unknown inspect failure must fail closed: {result:?}"
        );
        assert_eq!(
            fs::read_to_string(&invocation).unwrap(),
            format!("--state-dir {} --op stop\n", state_dir.display()),
            "the regression must exercise the adapter's real stop operation"
        );
        assert!(
            !env_dir.join("runtime-stopped").exists(),
            "unknown runtime state must not write a preservation receipt"
        );
        assert!(
            !state_dir.join("kill.log").exists(),
            "a failed stop must not be logged as complete"
        );
        let retained = registry.get("C1").unwrap();
        assert_eq!(
            retained.status,
            EnvironmentStatus::Retained,
            "unknown runtime state must emit no Preserved transition"
        );
        assert!(
            retained
                .last_error
                .as_deref()
                .is_some_and(|error| error.contains("could not verify")),
            "the retryable diagnostic must remain actionable: {:?}",
            retained.last_error
        );
        assert!(
            calls(&log).iter().all(|call| !call.starts_with("rm ")),
            "unknown runtime state must not attempt removal"
        );
    }
}

#[test]
fn a_recorded_container_is_removed_by_its_name_and_a_gone_directory_by_the_default() {
    let temp = tempfile::tempdir().unwrap();
    let bin = temp.path().join("bin");
    let log = temp.path().join("podman.log");
    fake_podman(&bin, &log);
    let state_dir = temp.path().join("state");
    let env_dir = state_dir.join("env-named");
    fs::create_dir_all(&env_dir).unwrap();
    fs::write(env_dir.join("container"), "custom-name\n").unwrap();
    assert!(
        run_kill(&bin, &state_dir, "env-named", "cleanup")
            .status
            .success()
    );
    assert!(
        run_kill(&bin, &state_dir, "env-gone", "kill")
            .status
            .success()
    );
    assert_eq!(
        calls(&log),
        ["rm -f --time 1 custom-name", "rm -f quecto-env-gone"]
    );
}
