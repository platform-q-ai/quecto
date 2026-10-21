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

/// #2659 review round 2: git listing the name while `remote get-url`
/// fails says nothing of a missing remote: the store is out of reach.
#[test]
fn a_listed_remote_git_cannot_name_is_out_of_reach() {
    let dir = tempfile::tempdir().unwrap();
    init(dir.path());
    let added = Command::new("git")
        .args(["remote", "add", "origin", "../remote.git"])
        .current_dir(dir.path())
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(added.success());
    // A git on PATH that fails `remote get-url` and runs every other call.
    let real = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let real = String::from_utf8(real.stdout).unwrap().trim().to_string();
    let bin = dir.path().join("fake-bin");
    std::fs::create_dir(&bin).unwrap();
    let script = format!(
        "#!/bin/sh\ncase \" $* \" in *\" get-url \"*) echo 'error: no' >&2; exit 128;; esac\nexec '{real}' \"$@\"\n"
    );
    let fake = bin.join("git");
    std::fs::write(&fake, script).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let vars: Vec<(OsString, OsString)> = vec![
        ("PATH".into(), path.into()),
        ("HOME".into(), dir.path().into()),
        ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
    ];
    let git = Git::reader(dir.path(), GitEnvironment::allowlisted(vars)).unwrap();
    let opened = GitBoardStore::new(git, "origin").map(|_| ());
    assert!(
        matches!(&opened, Err(StoreOpenError::Unavailable(BoardStoreError::Unavailable(why)))
            if why.contains("could not say")),
        "{opened:?}"
    );
}
