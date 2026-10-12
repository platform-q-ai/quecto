use super::*;
use crate::interface::tools::project_board::run;
use serde_json::{Value, json};
use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

fn sh(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .envs([
            ("HOME", "/nonexistent"),
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
        ])
        .envs([("GIT_AUTHOR_NAME", "T"), ("GIT_AUTHOR_EMAIL", "t@x.io")])
        .envs([
            ("GIT_COMMITTER_NAME", "T"),
            ("GIT_COMMITTER_EMAIL", "t@x.io"),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A clone of a bare remote whose committer is configured in the clone.
fn repository() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    sh(
        dir.path(),
        &["init", "-q", "--bare", "-b", "main", "remote.git"],
    );
    sh(dir.path(), &["clone", "-q", "remote.git", "work"]);
    let work = dir.path().join("work");
    sh(&work, &["config", "user.name", "Ada"]);
    sh(&work, &["config", "user.email", "ada@example.com"]);
    (dir, work)
}

fn environment(repo: &Path, access: Access) -> ProjectBoardEnvironment {
    let vars: Vec<(OsString, OsString)> = vec![
        ("PATH".into(), std::env::var_os("PATH").unwrap()),
        ("HOME".into(), repo.into()),
        ("GIT_CONFIG_GLOBAL".into(), "/dev/null".into()),
        ("GIT_CONFIG_NOSYSTEM".into(), "1".into()),
    ];
    ProjectBoardEnvironment {
        repo: repo.to_path_buf(),
        remote: "origin".into(),
        access,
        vars,
        cancelled: None,
    }
}

/// The real graph over a real remote: bootstrap, create, list, show, from
/// the index and the git store, as the committer the clone configures.
#[test]
fn the_board_runs_end_to_end_over_git_and_the_index() {
    let (_dir, work) = repository();
    let handles = build_project_board_handles(&environment(&work, Access::Write)).unwrap();
    let ops = |request: Value| run(&handles.operations, &request.to_string());
    let bootstrap = ops(json!({"op": "bootstrap"}));
    assert_eq!(
        (&bootstrap["ok"], &bootstrap["created"]),
        (&json!(true), &json!(true)),
        "{bootstrap}"
    );
    let created = ops(json!({"op": "create", "task": "a", "title": "A", "status": "ready"}));
    assert_eq!(created["ok"], json!(true), "{created}");
    assert_eq!(
        created["clock"]["referenced"],
        json!(false),
        "a local remote has no outside time"
    );
    let claimed = ops(json!({"op": "move", "task": "a", "to": "claimed"}));
    assert_eq!(
        claimed["task"]["claim"]["holder"],
        json!({"name": "Ada", "email": "ada@example.com"})
    );
    let listed = ops(json!({"op": "list"}));
    assert_eq!(listed["tasks"][0]["status"], json!("claimed"), "{listed}");
    let complete = ops(json!({"op": "complete", "task": "a"}));
    assert_eq!(
        complete["error"]["code"],
        json!("no_pull_request_host"),
        "{complete}"
    );
}

#[test]
fn a_read_needs_no_committer_and_a_write_says_none_is_configured() {
    let (_dir, work) = repository();
    let writer = build_project_board_handles(&environment(&work, Access::Write)).unwrap();
    run(&writer.operations, r#"{"op": "bootstrap"}"#);
    run(
        &writer.operations,
        r#"{"op": "create", "task": "a", "title": "A"}"#,
    );
    sh(&work, &["config", "--unset", "user.name"]);
    let reader = build_project_board_handles(&environment(&work, Access::Read)).unwrap();
    let listed = run(&reader.operations, r#"{"op": "list"}"#);
    assert_eq!(listed["tasks"][0]["id"], json!("a"), "{listed}");
    let refused = build_project_board_handles(&environment(&work, Access::Write));
    assert!(
        matches!(&refused, Err(BuildError::NoCommitter(why)) if why.contains("user.name")),
        "{:?}",
        refused.as_ref().err()
    );
    let mut nowhere = environment(&work, Access::Read);
    nowhere.remote = "nowhere".into();
    assert!(matches!(
        build_project_board_handles(&nowhere),
        Err(BuildError::Unavailable(_))
    ));
}

/// A directory git cannot use as a repository fails with that cause, not
/// as one with no committer configured.
#[test]
fn a_directory_that_is_no_repository_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = environment(dir.path(), Access::Write);
    // Discovery stops at the temporary directory: a repository above it
    // (a stray `/tmp/.git`) is never found.
    let parent = dir.path().parent().unwrap().as_os_str().to_owned();
    env.vars.push(("GIT_CEILING_DIRECTORIES".into(), parent));
    let refused = build_project_board_handles(&env);
    assert!(
        matches!(&refused, Err(BuildError::Unavailable(why)) if why.contains("rev-parse")),
        "{:?}",
        refused.as_ref().err()
    );
}

/// Only "not set" is a missing committer: git failing to read its
/// configuration, or a repository git cannot run in, is the store's
/// failure, with its cause.
#[test]
fn a_committer_git_cannot_read_is_not_a_missing_one() {
    let (_dir, work) = repository();
    let config = work.join(".git/config");
    let kept = std::fs::read_to_string(&config).unwrap();
    std::fs::write(&config, format!("{kept}\n[broken\n")).unwrap();
    let broken = build_project_board_handles(&environment(&work, Access::Write));
    assert!(
        matches!(&broken, Err(BuildError::Unavailable(why)) if why.contains("config")),
        "{:?}",
        broken.as_ref().err()
    );
    std::fs::write(&config, kept).unwrap();
    let mut elsewhere = environment(&work, Access::Write);
    elsewhere.repo = work.join("no-such-directory");
    assert!(
        matches!(
            build_project_board_handles(&elsewhere),
            Err(BuildError::Unavailable(_))
        ),
        "a repository git cannot run in"
    );
    sh(&work, &["config", "user.email", "not an email"]);
    let refused = build_project_board_handles(&environment(&work, Access::Write));
    assert!(
        matches!(&refused, Err(BuildError::NoCommitter(_))),
        "{:?}",
        refused.as_ref().err()
    );
}

/// A remote on GitHub with no `gh` to ask reads no pull requests, and
/// says why: there is no gh on the PATH. One on no such host says that.
#[test]
fn a_github_remote_with_no_gh_says_so() {
    let dir = tempfile::tempdir().unwrap();
    // git's own directory, every program but gh, as a PATH might be.
    let no_gh = dir.path().join("bin");
    std::fs::create_dir(&no_gh).unwrap();
    let git_dir = which("git").parent().unwrap().to_path_buf();
    for entry in std::fs::read_dir(&git_dir).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() != "gh" {
            std::os::unix::fs::symlink(entry.path(), no_gh.join(entry.file_name())).unwrap();
        }
    }
    let vars = vec![(OsString::from("PATH"), no_gh.into_os_string())];
    let missing = pull_request_states("https://github.com/o/r.git", &vars).err();
    assert!(
        matches!(&missing, Some(NoPullRequestHost::Unreachable(why)) if why.contains("no gh on the PATH")),
        "{missing:?}"
    );
    let elsewhere = pull_request_states("/srv/r.git", &vars).err();
    assert_eq!(elsewhere, Some(NoPullRequestHost::NotAPullRequestHost));
}

/// Where `program` is on this process's PATH.
fn which(program: &str) -> std::path::PathBuf {
    std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join(program))
        .find(|path| path.is_file())
        .unwrap_or_else(|| panic!("no {program} on the PATH"))
}

/// The call's cancel flag reaches the composed git store (#2513): a write
/// of a cancelled call pushes nothing.
#[test]
fn a_cancelled_calls_write_lands_nothing() {
    let (_dir, work) = repository();
    let writer = build_project_board_handles(&environment(&work, Access::Write)).unwrap();
    run(&writer.operations, r#"{"op": "bootstrap"}"#);
    let mut cancelled = environment(&work, Access::Write);
    cancelled.cancelled = Some(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
        true,
    )));
    let handles = build_project_board_handles(&cancelled).unwrap();
    let refused = run(
        &handles.operations,
        r#"{"op": "create", "task": "a", "title": "A"}"#,
    );
    assert_eq!(refused["ok"], json!(false), "{refused}");
    assert!(refused.to_string().contains("cancelled"), "{refused}");
    let listed = run(&writer.operations, r#"{"op": "list"}"#);
    assert_eq!(listed["tasks"], json!([]), "{listed}");
}
