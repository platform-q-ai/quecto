//! The Docker adapter distinguishes affirmative runtime absence from uncertain failures.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn fake_runtime(bin: &Path) {
    fs::create_dir_all(bin).unwrap();
    let script = bin.join("podman");
    fs::write(
        &script,
        r#"#!/usr/bin/env bash
case "$1" in
inspect)
  case "${FAKE_INSPECT:-missing}" in
    running) echo 'true 0 false' ;;
    stopped) echo 'false 17 false' ;;
    *) echo 'runtime transport failed' >&2; exit 125 ;;
  esac ;;
ps)
  [ "${FAKE_LIST_ERROR:-0}" = 0 ] || { echo 'daemon unavailable' >&2; exit 125; }
  [ "${FAKE_PRESENT:-0}" = 0 ] || printf '%s\n' 'quecto-env'
  ;;
esac
"#,
    )
    .unwrap();
    fs::set_permissions(script, fs::Permissions::from_mode(0o755)).unwrap();
}

fn inspect(bin: &Path, state: &Path, vars: &[(&str, &str)]) -> std::process::Output {
    let mut command = Command::new(root().join("scripts/container-runtime/docker/inspect.sh"));
    command
        .args(["--state-dir"])
        .arg(state)
        .env(
            "PATH",
            format!(
                "{}:{}",
                bin.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("QUECTO_CONTAINER_CLI", "podman")
        .env("QUECTO_CONTAINER_ENVIRONMENT_ID", "env");
    for (key, value) in vars {
        command.env(key, value);
    }
    command.output().unwrap()
}

#[test]
fn missing_container_is_dead_only_after_successful_inventory_proves_absence() {
    let t = tempfile::tempdir().unwrap();
    let bin = t.path().join("bin");
    fake_runtime(&bin);
    let state = t.path().join("state");
    fs::create_dir_all(&state).unwrap();
    let out = inspect(&bin, &state, &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("environment-removed"));
}

#[test]
fn inspect_failure_for_a_present_container_propagates() {
    let t = tempfile::tempdir().unwrap();
    let bin = t.path().join("bin");
    fake_runtime(&bin);
    let state = t.path().join("state");
    let env = state.join("env");
    fs::create_dir_all(&env).unwrap();
    fs::write(env.join("container"), "quecto-env\n").unwrap();
    let out = inspect(&bin, &state, &[("FAKE_PRESENT", "1")]);
    assert!(!out.status.success());
}

#[test]
fn unreadable_runtime_inventory_propagates_instead_of_claiming_absence() {
    let t = tempfile::tempdir().unwrap();
    let bin = t.path().join("bin");
    fake_runtime(&bin);
    let state = t.path().join("state");
    fs::create_dir_all(&state).unwrap();
    let out = inspect(&bin, &state, &[("FAKE_LIST_ERROR", "1")]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("daemon unavailable"));
}
