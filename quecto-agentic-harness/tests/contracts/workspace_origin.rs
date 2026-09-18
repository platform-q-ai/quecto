//! Contract for the `WorkspaceOrigin` port (#2024 S4e): a checkout's
//! `origin` remote URL exactly as git reports it; a directory that is not
//! a checkout, a checkout without `origin`, and a missing directory are
//! `None`, never an error; the answer is a read (nothing is written,
//! no prompt).
use std::path::Path;
use std::sync::Arc;

use quecto::application::environments::ports::WorkspaceOrigin;
use quecto::composition::standard_container::build_workspace_origin;

fn port() -> Arc<dyn WorkspaceOrigin> {
    build_workspace_origin()
}

fn git(dir: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
    assert!(
        status.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&status.stderr)
    );
}

#[test]
fn the_origin_is_read_as_git_reports_it() {
    let dir = tempfile::TempDir::new().unwrap();
    git(dir.path(), &["init", "-q"]);
    git(
        dir.path(),
        &["remote", "add", "origin", "git@example.test:org/repo.git"],
    );
    git(
        dir.path(),
        &["remote", "add", "upstream", "https://example.test/upstream"],
    );
    assert_eq!(
        port().origin(dir.path()).unwrap().as_deref(),
        Some("git@example.test:org/repo.git")
    );
}

#[test]
fn no_checkout_no_origin_and_no_directory_are_none() {
    let dir = tempfile::TempDir::new().unwrap();
    assert_eq!(port().origin(dir.path()).unwrap(), None);
    git(dir.path(), &["init", "-q"]);
    assert_eq!(port().origin(dir.path()).unwrap(), None);
    git(
        dir.path(),
        &["remote", "add", "upstream", "https://example.test/upstream"],
    );
    assert_eq!(
        port().origin(dir.path()).unwrap(),
        None,
        "only `origin` counts"
    );
    assert_eq!(port().origin(&dir.path().join("missing")).unwrap(), None);
}

#[test]
fn a_git_dir_in_the_environment_does_not_redirect_the_question() {
    // Another repository named by GIT_DIR (as a sibling test or a hook
    // may leave in the process environment) must not answer for the
    // checkout asked about.
    let other = tempfile::TempDir::new().unwrap();
    git(other.path(), &["init", "-q"]);
    git(
        other.path(),
        &["remote", "add", "origin", "https://example.test/other"],
    );
    let dir = tempfile::TempDir::new().unwrap();
    git(dir.path(), &["init", "-q"]);
    // The adapter under test must ignore it regardless of who else reads it.
    // SAFETY: the variable is set and removed within this test.
    unsafe { std::env::set_var("GIT_DIR", other.path().join(".git")) };
    let answer = port().origin(dir.path());
    // SAFETY: restores the environment this test changed above.
    unsafe { std::env::remove_var("GIT_DIR") };
    assert_eq!(answer.unwrap(), None);
}
