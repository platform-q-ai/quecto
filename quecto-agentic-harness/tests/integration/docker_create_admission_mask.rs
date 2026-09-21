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

/// How HOME and the admission directory are spelled on the host.
#[derive(Clone, Copy, PartialEq)]
enum Layout {
    /// HOME is a real directory; the admission dir is spelled under it.
    Plain,
    /// HOME is a symlink and the admission dir is spelled through the SAME
    /// alias — what the harness produces, since it derives the path from HOME.
    AliasedHome,
    /// HOME is a symlink but the admission dir uses the canonical spelling:
    /// the mask would land beside the identity mount, not over the authority.
    AliasedHomeCanonicalAdmission,
    /// The admission root is reached through a symlink inside ~/.quecto.
    LinkInsideQuecto,
}

fn run(admission_suffix: &str) -> (std::process::Output, PathBuf, String) {
    run_with_layout(admission_suffix, Layout::Plain)
}

fn assert_preflight_only(log: &Path) {
    let calls = fs::read_to_string(log).expect("mandatory runtime preflight must be recorded");
    let calls: Vec<_> = calls.lines().collect();
    assert!(!calls.is_empty(), "mandatory runtime preflight must run");
    assert!(
        calls
            .iter()
            .all(|call| ["image exists ", "image inspect ", "run --rm "]
                .iter()
                .any(|allowed| call.starts_with(allowed))),
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
    layout: Layout,
) -> (std::process::Output, PathBuf, String) {
    let symlink_home = matches!(
        layout,
        Layout::AliasedHome | Layout::AliasedHomeCanonicalAdmission
    );
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
    let admission = match layout {
        Layout::AliasedHomeCanonicalAdmission => real_home.join(admission_suffix),
        Layout::Plain | Layout::AliasedHome => home.join(admission_suffix),
        Layout::LinkInsideQuecto => {
            // ~/.quecto/linked -> ~/.quecto/real-authority; the suffix's leaf
            // is the client directory beneath the link.
            let leaf = Path::new(admission_suffix).file_name().unwrap();
            let real = home.join(".quecto/real-authority");
            fs::create_dir_all(real.join(leaf)).unwrap();
            std::os::unix::fs::symlink(&real, home.join(".quecto/linked")).unwrap();
            home.join(".quecto/linked").join(leaf)
        }
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
if [ "$1" = image ] && [ "$2" = inspect ]; then printf '\n'; exit 0; fi
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
fn a_symlinked_home_works_when_the_admission_dir_is_spelled_through_the_same_alias() {
    // What the harness produces on a host whose HOME is a symlink (it derives
    // the admission dir from HOME): identity mount and mask agree, so the
    // launch must succeed and the mask must sit under the alias spelling.
    let (out, log, admission) = run_with_layout(".quecto/admission/client", Layout::AliasedHome);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let calls = fs::read_to_string(log).unwrap();
    let root = Path::new(&admission)
        .parent()
        .unwrap()
        .display()
        .to_string();
    assert!(root.contains("alias-home"), "{root}");
    let ro = calls.find(&format!("{root}:ro")).expect("mask mounted");
    let rw = calls
        .find(&format!("{admission}:rw"))
        .expect("client mounted");
    assert!(ro < rw);
}

#[test]
fn rejects_a_mask_that_would_miss_the_identity_mount() {
    // Canonical spelling under an aliased HOME, and a symlink inside
    // ~/.quecto: in both the read-only mask would land beside the authority
    // instead of over it. Refused before anything is allocated.
    for (layout, message) in [
        (
            Layout::AliasedHomeCanonicalAdmission,
            "is not spelled under",
        ),
        (
            Layout::LinkInsideQuecto,
            "through a symbolic link inside ~/.quecto",
        ),
    ] {
        let (out, log, _) = run_with_layout(".quecto/admission/client", layout);
        assert!(!out.status.success());
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(message), "{stderr}");
        assert_preflight_only(&log);
        assert_state_empty(&log);
    }
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
