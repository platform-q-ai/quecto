#![cfg(unix)]

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
fn executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut p = fs::metadata(path).unwrap().permissions();
    p.set_mode(0o755);
    fs::set_permissions(path, p).unwrap();
}

fn run(admission_suffix: &str) -> (std::process::Output, PathBuf, String) {
    run_with_layout(admission_suffix, false)
}

fn assert_preflight_only(log: &Path) {
    let calls = fs::read_to_string(log).expect("mandatory runtime preflight must be recorded");
    let calls: Vec<_> = calls.lines().collect();
    assert!(!calls.is_empty(), "mandatory runtime preflight must run");
    assert!(
        calls
            .iter()
            .all(|call| { call.starts_with("image exists ") || call.starts_with("run --rm ") }),
        "rejected layout may invoke only allowlisted preflights, got:\n{calls}",
        calls = calls.join("\n")
    );
}

fn assert_state_empty(log: &Path) {
    let state = log.parent().unwrap().join("state");
    assert_eq!(fs::read_dir(state).unwrap().count(), 0);
}

fn run_with_layout(
    admission_suffix: &str,
    symlink_home: bool,
) -> (std::process::Output, PathBuf, String) {
    let t = tempfile::tempdir().unwrap();
    let base = t.keep();
    let real_home = base.join("real-home");
    let home = if symlink_home {
        let alias = base.join("alias-home");
        fs::create_dir_all(&real_home).unwrap();
        std::os::unix::fs::symlink(&real_home, &alias).unwrap();
        alias
    } else {
        real_home.clone()
    };
    let state = base.join("state");
    let socket_dir = base.join("run");
    let bin = base.join("bin");
    for d in [&home.join(".quecto"), &state, &socket_dir, &bin] {
        fs::create_dir_all(d).unwrap();
    }
    let admission = if symlink_home {
        real_home.join(admission_suffix)
    } else {
        home.join(admission_suffix)
    };
    fs::create_dir_all(&admission).unwrap();
    let log = base.join("runtime.log");
    let podman = bin.join("podman");
    executable(
        &podman,
        &format!(
            r#"#!/usr/bin/env bash
printf '%q ' "$@" >> {log:?}; printf '\n' >> {log:?}
if [ "$1" = image ] && [ "$2" = exists ]; then exit 0; fi
if [ "$1" = run ] && [ "${{2:-}}" = --rm ]; then exit 0; fi
if [ "$1" = run ]; then
  args=("$@"); for ((i=0;i<${{#args[@]}};i++)); do
    if [ "${{args[i]}}" = -v ]; then
      spec="${{args[i+1]}}"; src="${{spec%%:*}}"; dstmode="${{spec#*:}}"
      if [[ "$dstmode" = {admission:?}:rw ]]; then [ -d "$mask/$leaf" ] || exit 91; fi
      if [[ "$dstmode" = *:ro && "$src" = */admission-mask ]]; then mask="$src"; leaf={leaf:?}; fi
    fi
  done
  printf 'container-id\n'; exit 0
fi
exit 125
"#,
            log = log,
            admission = admission,
            leaf = admission.file_name().unwrap().to_string_lossy()
        ),
    );
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let output = Command::new(root().join("scripts/container-runtime/docker/create.sh"))
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "--",
            "/bin/true",
            "--socket",
            socket_dir.join("child.sock").to_str().unwrap(),
        ])
        .env("PATH", path)
        .env("HOME", &home)
        .env("QUECTO_CONTAINER_CLI", "podman")
        .env("QUECTO_CONTAINER_ENVIRONMENT_REF", "test")
        .env("QUECTO_ADMISSION_DIR", &admission)
        .output()
        .unwrap();
    (output, log, admission.to_string_lossy().into_owned())
}

#[test]
fn rejection_oracles_detect_container_launch_and_state_allocation() {
    let base = tempfile::tempdir().unwrap();
    let log = base.path().join("runtime.log");
    fs::write(
        &log,
        "image exists quecto-agent:latest\nrun --name forbidden\n",
    )
    .unwrap();
    assert!(std::panic::catch_unwind(|| assert_preflight_only(&log)).is_err());

    let state = base.path().join("state");
    fs::create_dir(&state).unwrap();
    fs::write(state.join("leaked-allocation"), "").unwrap();
    assert!(std::panic::catch_unwind(|| assert_state_empty(&log)).is_err());
}

#[test]
fn rejects_symlinked_home_when_admission_uses_canonical_spelling() {
    let (out, log, _) = run_with_layout(".quecto/admission/client", true);
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr)
            .contains("HOME/.quecto must be a normalized canonical path"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_preflight_only(&log);
    assert_state_empty(&log);
}

#[test]
fn rejects_identity_root_client_before_allocating_environment() {
    let (out, log, _) = run(".quecto/client");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("identity-mounted ~/.quecto itself"));
    assert_preflight_only(&log);
    assert_state_empty(&log);
}

#[test]
fn masked_authority_precreates_child_before_read_only_parent_and_rw_child() {
    for suffix in [
        ".quecto/admission/client",
        ".quecto/custom/peer",
        ".quecto/deeper/authority/client",
    ] {
        let (out, log, admission) = run(suffix);
        assert!(
            out.status.success(),
            "{}: {}",
            suffix,
            String::from_utf8_lossy(&out.stderr)
        );
        let calls = fs::read_to_string(log).unwrap();
        let root = Path::new(&admission)
            .parent()
            .unwrap()
            .display()
            .to_string();
        let ro = calls.find(&format!("{}:ro", root)).unwrap();
        let rw = calls.find(&format!("{}:rw", admission)).unwrap();
        assert!(ro < rw);
    }
}
