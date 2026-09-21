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
    /// HOME is exported with a trailing slash; the harness still spells the
    /// admission dir without the doubled slash (`Path::join` collapses it).
    TrailingSlashHome,
    /// The admission dir is spelled with a `//` and a `/./` and a trailing
    /// slash: the same directory, to be mounted at its normal form.
    UnnormalizedSpelling,
    /// The client leaf is a symlink to its own authority (`client -> .`).
    SymlinkedLeaf,
    /// The spelling climbs `..` out of a symlinked directory, so its lexical
    /// normal form names a different directory than the kernel would open.
    DotDotThroughLink,
    /// ~/.quecto is itself a symlink; the admission dir is spelled under it.
    QuectoIsSymlink,
    /// The admission dir lives outside ~/.quecto altogether.
    OutsideQuecto,
    /// The admission dir is handed over relative to the script's cwd (HOME).
    RelativeSpelling,
}

/// One run of the create script: the temp dir lives as long as this does.
struct Run {
    out: std::process::Output,
    log: PathBuf,
    /// The spelling handed to the script.
    admission: String,
    /// The spelling the script is expected to mount.
    mounted: String,
    _base: tempfile::TempDir,
}

fn run(admission_suffix: &str) -> Run {
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

fn run_with_layout(admission_suffix: &str, layout: Layout) -> Run {
    let symlink_home = matches!(
        layout,
        Layout::AliasedHome | Layout::AliasedHomeCanonicalAdmission
    );
    let t = tempfile::tempdir().unwrap();
    let base = t.path().to_path_buf();
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
    if layout == Layout::QuectoIsSymlink {
        let elsewhere = base.join("quecto-elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        fs::create_dir_all(&home).unwrap();
        std::os::unix::fs::symlink(&elsewhere, home.join(".quecto")).unwrap();
    }
    for d in [&home.join(".quecto"), &state, &socket_dir, &bin] {
        fs::create_dir_all(d).unwrap();
    }
    let admission = match layout {
        Layout::AliasedHomeCanonicalAdmission => real_home.join(admission_suffix),
        Layout::Plain
        | Layout::AliasedHome
        | Layout::TrailingSlashHome
        | Layout::UnnormalizedSpelling
        | Layout::RelativeSpelling
        | Layout::QuectoIsSymlink => home.join(admission_suffix),
        Layout::OutsideQuecto => base.join("elsewhere").join(admission_suffix),
        Layout::SymlinkedLeaf => {
            let authority = home.join(admission_suffix).parent().unwrap().to_path_buf();
            fs::create_dir_all(&authority).unwrap();
            let leaf = home.join(admission_suffix);
            std::os::unix::fs::symlink(".", &leaf).unwrap();
            leaf
        }
        Layout::DotDotThroughLink => {
            // ~/.quecto/hop -> <base>/far/away, and the spelling is
            // ~/.quecto/hop/../admission/client: lexically that is
            // ~/.quecto/admission/client, but the kernel opens
            // <base>/far/admission/client.
            fs::create_dir_all(base.join("far/away")).unwrap();
            fs::create_dir_all(
                base.join("far")
                    .join(Path::new(admission_suffix).strip_prefix(".quecto").unwrap()),
            )
            .unwrap();
            std::os::unix::fs::symlink(base.join("far/away"), home.join(".quecto/hop")).unwrap();
            fs::create_dir_all(home.join(admission_suffix)).unwrap();
            home.join(".quecto/hop/..")
                .join(Path::new(admission_suffix).strip_prefix(".quecto").unwrap())
        }
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
    let mounted = admission.to_string_lossy().into_owned();
    let admission = match layout {
        Layout::UnnormalizedSpelling => {
            let parent = admission.parent().unwrap().display().to_string();
            let leaf = admission
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            PathBuf::from(format!("{parent}//./{leaf}/"))
        }
        Layout::RelativeSpelling => PathBuf::from(admission_suffix),
        _ => admission,
    };
    let mounted_root = Path::new(&mounted).parent().unwrap().to_path_buf();
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
  mask=""; client=0
  args=("$@"); for ((i=0;i<${{#args[@]}};i++)); do
    if [ "${{args[i]}}" = -v ]; then
      spec="${{args[i+1]}}"; src="${{spec%%:*}}"; dstmode="${{spec#*:}}"
      if [[ "$src" = */admission-mask ]]; then
        # The mask covers exactly the authority root, read-only, and holds
        # nothing but the owner-only client mountpoint.
        [ "$dstmode" = {mounted_root:?}:ro ] || exit 93
        [ "$(ls -A "$src")" = {leaf:?} ] || exit 94
        [ "$(stat -c %a "$src/"{leaf:?})" = 700 ] || exit 95
        mask="$src"
      fi
      if [ "$dstmode" = {mounted:?}:rw ]; then
        [ "$src" = {mounted:?} ] || exit 96
        if [ {masked} = 1 ]; then
          [ -n "$mask" ] || exit 92
          [ -d "$mask/"{leaf:?} ] || exit 91
        fi
        client=1
      fi
    fi
  done
  [ "$client" = 1 ] || exit 97
  if [ {masked} = 0 ] && [ -n "$mask" ]; then exit 98; fi
  printf 'container-id\n'; exit 0
fi
exit 125
"#,
            log = log,
            mounted = mounted,
            mounted_root = mounted_root,
            masked = u8::from(layout != Layout::OutsideQuecto),
            leaf = Path::new(&mounted).file_name().unwrap().to_string_lossy()
        ),
    );
    // Nothing of the developer's reaches the run: a real create writes any
    // provider key and the `gh auth token` it finds to its provider-env.
    executable(&bin.join("gh"), "#!/bin/sh\nexit 1\n");
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let home_env = match layout {
        Layout::TrailingSlashHome => format!("{}/", home.display()),
        _ => home.display().to_string(),
    };
    let mut command = Command::new(root().join("scripts/container-runtime/docker/create.sh"));
    command
        .current_dir(&home)
        .args([
            "--state-dir",
            state.to_str().unwrap(),
            "--",
            "/bin/true",
            "--socket",
            socket_dir.join("child.sock").to_str().unwrap(),
        ])
        .env("PATH", path)
        .env("HOME", home_env)
        .env("QUECTO_CONTAINER_CLI", "podman")
        .env("QUECTO_CONTAINER_ENVIRONMENT_REF", "test")
        .env("QUECTO_ADMISSION_DIR", &admission);
    for inherited in [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "OPENROUTER_API_KEY",
        "FIREWORKS_API_KEY",
        "GH_TOKEN",
        "GITHUB_TOKEN",
        "QUECTO_DOCKER_IMAGE",
        "QUECTO_CONTAINER_PIDS_LIMIT",
        "QUECTO_REPO_CHECK_TIMEOUT",
    ] {
        command.env_remove(inherited);
    }
    Run {
        out: command.output().unwrap(),
        log,
        admission: admission.to_string_lossy().into_owned(),
        mounted,
        _base: t,
    }
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

/// The launch succeeded with the mask over the authority root and the
/// client re-exposed beneath it (the fake runtime refuses anything else).
fn assert_masked_launch(run: &Run) {
    assert!(
        run.out.status.success(),
        "{}: {}",
        run.admission,
        String::from_utf8_lossy(&run.out.stderr)
    );
    let calls = fs::read_to_string(&run.log).unwrap();
    // The fake logs its argv with `%q`: a space is written `\ `.
    let mounted = run.mounted.replace(' ', "\\ ");
    let root = Path::new(&mounted).parent().unwrap().display().to_string();
    assert!(calls.contains(&format!("{root}:ro")), "{calls}");
    assert!(
        calls.contains(&format!("{mounted}:{mounted}:rw")),
        "{calls}"
    );
}

fn assert_refused(run: &Run, message: &str) {
    assert!(!run.out.status.success(), "{}", run.admission);
    let stderr = String::from_utf8_lossy(&run.out.stderr);
    assert!(stderr.contains(message), "{}: {stderr}", run.admission);
    assert_preflight_only(&run.log);
    assert_state_empty(&run.log);
}

#[test]
fn a_symlinked_home_works_when_the_admission_dir_is_spelled_through_the_same_alias() {
    // What the harness produces on a host whose HOME is a symlink (it derives
    // the admission dir from HOME): identity mount and mask agree, so the
    // launch must succeed and the mask must sit under the alias spelling.
    let run = run_with_layout(".quecto/admission/client", Layout::AliasedHome);
    assert!(run.mounted.contains("alias-home"), "{}", run.mounted);
    assert_masked_launch(&run);
}

#[test]
fn spellings_of_the_same_directory_are_mounted_at_their_normal_form() {
    // A HOME exported with a trailing slash, a `//`, a `/./`, a trailing
    // slash on the dir, ~/.quecto being a symlink: the same directory, so
    // the launch proceeds and mounts the normal form it classified.
    for layout in [
        Layout::TrailingSlashHome,
        Layout::UnnormalizedSpelling,
        Layout::QuectoIsSymlink,
    ] {
        let run = run_with_layout(".quecto/admission/client", layout);
        assert_masked_launch(&run);
    }
    let run = run_with_layout(".quecto/admission/client", Layout::UnnormalizedSpelling);
    assert_ne!(run.admission, run.mounted);
}

#[test]
fn the_record_exec_compares_keeps_the_configured_spelling() {
    let run = run_with_layout(".quecto/admission/client", Layout::UnnormalizedSpelling);
    assert_masked_launch(&run);
    let state = run.log.parent().unwrap().join("state");
    let env_dir = fs::read_dir(state)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.is_dir())
        .expect("one environment directory");
    let recorded = fs::read_to_string(env_dir.join("admission-dir")).unwrap();
    assert_eq!(recorded.trim_end_matches('\n'), run.admission);
}

#[test]
fn an_admission_dir_outside_the_identity_mount_is_mounted_without_a_mask() {
    let run = run_with_layout("admission/client", Layout::OutsideQuecto);
    assert!(
        run.out.status.success(),
        "{}",
        String::from_utf8_lossy(&run.out.stderr)
    );
    let calls = fs::read_to_string(&run.log).unwrap();
    assert!(!calls.contains("admission-mask"), "{calls}");
    assert!(
        calls.contains(&format!("{0}:{0}:rw", run.mounted)),
        "{calls}"
    );
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
        assert_refused(
            &run_with_layout(".quecto/admission/client", layout),
            message,
        );
    }
}

#[test]
fn rejects_a_client_leaf_that_is_a_symbolic_link() {
    // `client -> .` would make the read-write mount expose the authority
    // (journal, admin socket, token) on top of the read-only mask.
    assert_refused(
        &run_with_layout(".quecto/admission/client", Layout::SymlinkedLeaf),
        "is a symbolic link; the client directory must be a real directory",
    );
}

#[test]
fn rejects_a_spelling_whose_normal_form_names_another_directory() {
    assert_refused(
        &run_with_layout(".quecto/admission/client", Layout::DotDotThroughLink),
        "does not name the same directory once normalized",
    );
}

#[test]
fn rejects_a_relative_path_and_a_control_character() {
    assert_refused(
        &run_with_layout(".quecto/admission/client", Layout::RelativeSpelling),
        "must be an absolute path",
    );
    // `dirname` in a command substitution would drop the trailing newline
    // and classify `admission` while the runtime mounts `admission\n`.
    assert_refused(
        &run(".quecto/admission\n/client"),
        "printable characters only",
    );
}

#[test]
fn rejects_identity_root_client_before_allocating_environment() {
    assert_refused(&run(".quecto/client"), "identity-mounted ~/.quecto itself");
}

#[test]
fn masked_authority_precreates_child_before_read_only_parent_and_rw_child() {
    for suffix in [
        ".quecto/admission/client",
        ".quecto/custom/peer",
        ".quecto/deeper/authority/client",
        ".quecto/admission/-leading-dash",
        ".quecto/admission/with space",
    ] {
        assert_masked_launch(&run(suffix));
    }
}
