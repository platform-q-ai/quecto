//! The official Docker adapter's `kill.sh` against a fake runtime CLI
//! (#2024 S4d, round 2 F-C of #2033): what it asks the runtime to remove.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

/// A `podman` on a controlled PATH that records every argv line it is
/// called with and succeeds.
fn fake_podman(bin: &Path, log: &Path) {
    fs::create_dir_all(bin).unwrap();
    let script = bin.join("podman");
    fs::write(
        &script,
        format!(
            "#!/usr/bin/env bash\nprintf '%s\\n' \"$*\" >>'{}'\n",
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
