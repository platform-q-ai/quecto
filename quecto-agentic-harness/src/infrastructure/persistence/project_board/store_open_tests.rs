//! #2659 review round 1: only a name git's own list of remotes lacks is
//! no such remote; git failing for any other reason is a store out of
//! reach.
use super::*;
use crate::infrastructure::persistence::project_board::git::GitEnvironment;
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

fn git_at(dir: &Path) -> Git {
    let vars: Vec<(OsString, OsString)> = vec![
        ("PATH".into(), std::env::var_os("PATH").unwrap()),
        ("HOME".into(), dir.into()),
        ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
    ];
    Git::reader(dir, GitEnvironment::allowlisted(vars)).unwrap()
}

fn init(dir: &Path) {
    let done = Command::new("git")
        .args(["init", "-q"])
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(done.success());
}

#[test]
fn a_remote_git_does_not_list_is_no_such_remote() {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path());
    let opened = GitBoardStore::new(git_at(dir.path()), "nope").map(|_| ());
    assert!(
        matches!(&opened, Err(StoreOpenError::NoSuchRemote(why)) if why.contains("nope")),
        "{opened:?}"
    );
}

#[test]
fn a_repository_git_cannot_read_is_out_of_reach() {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path());
    // A config git refuses to parse: every git call fails, `git remote`
    // too, so no name can be told missing.
    let git = git_at(dir.path());
    std::fs::write(
        dir.path().join(".git/config"),
        "[remote \"origin\"\n\tbroken",
    )
    .unwrap();
    let opened = GitBoardStore::new(git, "origin").map(|_| ());
    assert!(
        matches!(&opened, Err(StoreOpenError::Unavailable(BoardStoreError::Unavailable(why)))
            if why.contains("could not say")),
        "{opened:?}"
    );
}
