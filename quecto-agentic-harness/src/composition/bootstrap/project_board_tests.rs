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
    }
}

/// The real graph over a real remote: bootstrap, create, list, show, from
/// the index and the git store, as the committer the clone configures.
#[test]
fn the_board_runs_end_to_end_over_git_and_the_index() {
    let (_dir, work) = repository();
    let handles = build_project_board_handles(&environment(&work, Access::Write)).unwrap();
    let ops = |request: Value| run(&handles.operations, &request);
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
        json!("pull_requests_unavailable"),
        "{complete}"
    );
}

#[test]
fn a_read_needs_no_committer_and_a_write_says_none_is_configured() {
    let (_dir, work) = repository();
    let writer = build_project_board_handles(&environment(&work, Access::Write)).unwrap();
    run(&writer.operations, &json!({"op": "bootstrap"}));
    run(
        &writer.operations,
        &json!({"op": "create", "task": "a", "title": "A"}),
    );
    sh(&work, &["config", "--unset", "user.name"]);
    let reader = build_project_board_handles(&environment(&work, Access::Read)).unwrap();
    let listed = run(&reader.operations, &json!({"op": "list"}));
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
