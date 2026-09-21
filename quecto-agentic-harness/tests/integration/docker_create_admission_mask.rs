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
    let t = tempfile::tempdir().unwrap();
    let base = t.keep();
    let home = base.join("home");
    let state = base.join("state");
    let socket_dir = base.join("run");
    let bin = base.join("bin");
    for d in [&home.join(".quecto"), &state, &socket_dir, &bin] {
        fs::create_dir_all(d).unwrap();
    }
    let admission = home.join(admission_suffix);
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
