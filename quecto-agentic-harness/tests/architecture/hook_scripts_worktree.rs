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

/// Runs `bash -c script` in `dir` with git isolated from the user's config.
fn bash(dir: &Path, home: &Path, script: &str) -> Output {
    Command::new("bash")
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .env("HOME", home)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
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
