//! The hook installer, activator and checker agree on where hooks and the
//! `--no-verify` wrapper live, including in a linked worktree where `.git`
//! is a file rather than a directory (#2119).
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const SCRIPTS: [&str; 6] = [
    "install-hooks.sh",
    "activate-hooks.sh",
    "check-hooks-installed.sh",
    "git-wrapper.sh",
    "pre-commit.sh",
    "pre-push.sh",
];

/// Runs `bash -c script` in `dir` with git isolated from the user's config
/// and from any `GIT_*` state inherited from a hook that runs this test.
fn bash(dir: &Path, home: &Path, script: &str) -> Output {
    let mut command = Command::new("bash");
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            command.env_remove(name);
        }
    }
    command
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .output()
        .expect("bash runs")
}

fn assert_ok(what: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{what} failed ({}):\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A repository holding copies of the hook scripts, plus a linked worktree.
fn repository_with_worktree(tmp: &Path) -> (PathBuf, PathBuf) {
    let main = tmp.join("main");
    let scripts = main.join("scripts");
    std::fs::create_dir_all(&scripts).expect("scripts dir");
    for name in SCRIPTS {
        std::fs::copy(Path::new("../scripts").join(name), scripts.join(name))
            .unwrap_or_else(|e| panic!("copy {name}: {e}"));
    }
    let setup = bash(
        &main,
        tmp,
        "git init -q -b master . && git add scripts && git commit -qm scripts \
         && git worktree add -q ../linked -b linked",
    );
    assert_ok("repository setup", &setup);
    (main, tmp.join("linked"))
}

fn install_activate_and_check(dir: &Path, home: &Path) {
    let installed = bash(dir, home, "bash scripts/install-hooks.sh");
    assert_ok("install-hooks.sh", &installed);
    let checked = bash(
        dir,
        home,
        "source scripts/activate-hooks.sh && bash scripts/check-hooks-installed.sh",
    );
    assert_ok("activate + check-hooks-installed.sh", &checked);
}

#[test]
fn hooks_install_and_verify_in_a_linked_worktree() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (_, linked) = repository_with_worktree(tmp.path());
    assert!(
        linked.join(".git").is_file(),
        "a linked worktree's .git is a file"
    );

    install_activate_and_check(&linked, tmp.path());
}

#[test]
fn hooks_install_and_verify_in_the_main_checkout() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (main, _) = repository_with_worktree(tmp.path());
    assert!(main.join(".git").is_dir());

    install_activate_and_check(&main, tmp.path());
}

#[test]
fn one_activation_covers_every_worktree_of_the_repository() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (main, _) = repository_with_worktree(tmp.path());
    let checked = bash(
        &main,
        tmp.path(),
        "bash scripts/install-hooks.sh >/dev/null && source scripts/activate-hooks.sh \
         && cd ../linked && bash scripts/check-hooks-installed.sh",
    );
    assert_ok(
        "check in the linked worktree after activating in main",
        &checked,
    );
}

#[test]
fn a_configured_hooks_path_is_refused_and_left_untouched() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (main, _) = repository_with_worktree(tmp.path());
    let shared = tmp.path().join("shared-hooks");
    std::fs::create_dir_all(&shared).expect("shared hooks dir");
    std::fs::write(shared.join("pre-merge-commit"), "#!/bin/sh\nexit 0\n").expect("user hook");
    let configured = bash(
        &main,
        tmp.path(),
        &format!("git config --global core.hooksPath {}", shared.display()),
    );
    assert_ok("configure core.hooksPath", &configured);

    let installed = bash(&main, tmp.path(), "bash scripts/install-hooks.sh");

    assert!(!installed.status.success(), "install must refuse");
    assert!(
        String::from_utf8_lossy(&installed.stderr).contains("core.hooksPath"),
        "the refusal names the setting: {}",
        String::from_utf8_lossy(&installed.stderr)
    );
    let mut left: Vec<_> = std::fs::read_dir(&shared)
        .expect("read shared hooks")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    left.sort();
    assert_eq!(left, ["pre-merge-commit"], "the user's hooks are untouched");
}

#[test]
fn activating_outside_a_repository_fails_without_touching_path() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let loose = tmp.path().join("loose");
    std::fs::create_dir_all(loose.join("scripts")).expect("scripts dir");
    for name in ["activate-hooks.sh", "git-wrapper.sh"] {
        std::fs::copy(
            Path::new("../scripts").join(name),
            loose.join("scripts").join(name),
        )
        .expect("copy script");
    }
    let sourced = bash(
        &loose,
        tmp.path(),
        "before=\"$PATH\"; source scripts/activate-hooks.sh; status=$?; \
         [ \"$PATH\" = \"$before\" ] && echo unchanged; exit $status",
    );
    assert!(!sourced.status.success(), "activation must report failure");
    let stdout = String::from_utf8_lossy(&sourced.stdout);
    assert!(stdout.contains("unchanged"), "PATH changed: {stdout}");
    assert!(!stdout.contains("activated"), "no false success: {stdout}");
}
