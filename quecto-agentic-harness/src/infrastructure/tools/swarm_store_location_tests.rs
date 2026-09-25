use super::*;
use crate::infrastructure::tools::swarm_bridge::SwarmContext;
use std::path::Path;
use std::process::Command;

/// git, isolated from the user's configuration and from any repository the
/// test itself runs inside (a hook's GIT_DIR and friends).
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

/// A repository shaped like one with a committed standard container: its
/// `.quecto/` is tracked.
fn repository() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::create_dir_all(dir.path().join(".quecto/containers/standard")).unwrap();
    std::fs::write(
        dir.path().join(".quecto/containers/standard/Containerfile"),
        "FROM x\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("app.py"), "code\n").unwrap();
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "base"]);
    dir
}

fn context(checkout: &Path) -> SwarmContext {
    SwarmContext {
        checkout: checkout.to_path_buf(),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(crate::application::swarm::LifecycleService),
    }
}

/// Create a run as the container's creator does: make the store's directory,
/// then create the run there.
fn created_run(checkout: &Path) -> (SwarmContext, String) {
    let context = context(checkout);
    std::fs::create_dir_all(context.database().parent().unwrap()).unwrap();
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;
    let pid = std::process::id();
    context
        .create_run(
            &serde_json::json!({"goal":"g", "constraints":[], "criteria":[{"id":"t","kind":"command","description":"pass"}], "member_limit":2, "deadline":deadline}),
            &crate::domain::swarm::ProcessIdentity {
                pid,
                started: super::super::swarm_bridge::process_start(pid).unwrap(),
            },
            None,
        )
        .unwrap();
    let id = run_id(&context);
    assert_ne!(id, "null", "the run has an id");
    (context, id)
}

/// The run's id, as the store reports it.
fn run_id(context: &SwarmContext) -> String {
    context.call("_status", serde_json::json!([])).unwrap()["id"].to_string()
}

/// The board belongs to the checkout's own git directory whenever it has
/// one (a linked worktree's too); only a checkout with none keeps it in the
/// work tree.
#[test]
fn the_board_lives_in_the_checkouts_own_git_directory() {
    let repo = repository();
    assert_eq!(
        located(repo.path()),
        repo.path().join(".git/quecto/swarm.sqlite")
    );
    let worktrees = tempfile::tempdir().unwrap();
    let linked = worktrees.path().join("wt");
    git(
        repo.path(),
        &["worktree", "add", "-q", linked.to_str().unwrap()],
    );
    let own = repo.path().join(".git/worktrees/wt");
    assert_eq!(
        std::fs::canonicalize(located(&linked).parent().unwrap().parent().unwrap()).unwrap(),
        std::fs::canonicalize(own).unwrap()
    );
    let plain = tempfile::tempdir().unwrap();
    assert_eq!(
        located(plain.path()),
        plain.path().join(".quecto/swarm.sqlite")
    );
    std::fs::write(plain.path().join(".git"), "gitdir: /nowhere\n").unwrap();
    assert_eq!(
        located(plain.path()),
        plain.path().join(".quecto/swarm.sqlite")
    );
}

/// #2145: the git commands agents run on a checkout neither delete nor
/// replace the live board, even where a branch tracks a stale
/// `.quecto/swarm.sqlite`.
#[test]
fn the_board_survives_every_git_command_agents_run() {
    let repo = repository();
    git(repo.path(), &["checkout", "-q", "-b", "stale"]);
    std::fs::write(repo.path().join(".quecto/swarm.sqlite"), "stale board").unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-q", "-m", "committed a board"]);
    git(repo.path(), &["checkout", "-q", "main"]);
    let (context, id) = created_run(repo.path());
    std::fs::write(repo.path().join("app.py"), "changed\n").unwrap();
    for command in [
        &["stash", "-u"][..],
        &["clean", "-fdx"],
        &["checkout", "-q", "stale"],
        &["checkout", "-q", "main"],
        &["merge", "-q", "--no-edit", "stale"],
        &["reset", "-q", "--hard", "HEAD~1"],
        &["clean", "-fdX"],
    ] {
        git(repo.path(), command);
        let status = context.control_status();
        assert!(status.is_ok(), "after git {command:?}: {status:?}");
        assert_eq!(run_id(&context), id, "after git {command:?}");
    }
}

/// PR #2148 swarm review: in a linked worktree the board lives in that
/// worktree's git directory, so `git clean -fdx` there leaves it.
#[test]
fn a_linked_worktrees_board_survives_git_clean() {
    let repo = repository();
    let worktrees = tempfile::tempdir().unwrap();
    let linked = worktrees.path().join("wt");
    git(
        repo.path(),
        &["worktree", "add", "-q", linked.to_str().unwrap()],
    );
    let (context, id) = created_run(&linked);
    git(&linked, &["clean", "-fdx"]);
    assert_eq!(run_id(&context), id);
}

/// A repository that tracks a stale board: the location follows the layout,
/// so the stale file is never taken for this container's board.
#[test]
fn a_stale_board_committed_in_the_repository_is_never_adopted() {
    let repo = repository();
    std::fs::write(repo.path().join(".quecto/swarm.sqlite"), "stale board").unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-q", "-m", "committed a board"]);
    assert!(!context(repo.path()).run_created().unwrap());
}

/// PR #2148 swarm review: a layout that changes mid-run never moves a live
/// board. A checkout with no git directory gains one (and a store directory
/// in it): this process keeps its board; a new process finds no store there
/// and its member is refused rather than given another board.
#[test]
fn a_board_found_is_pinned_and_a_later_layout_change_fails_closed() {
    let plain = tempfile::tempdir().unwrap();
    let (context, id) = created_run(plain.path());
    git(plain.path(), &["init", "-q"]);
    std::fs::create_dir_all(plain.path().join(".git/quecto")).unwrap();
    assert_eq!(
        context.database(),
        plain.path().join(".quecto/swarm.sqlite")
    );
    assert_eq!(run_id(&context), id);
    // However the checkout is spelled, the pin holds.
    let links = tempfile::tempdir().unwrap();
    let spelled = links.path().join("checkout");
    std::os::unix::fs::symlink(plain.path(), &spelled).unwrap();
    let respelled = SwarmContext {
        checkout: spelled,
        ..context.clone()
    };
    assert_eq!(
        respelled.database(),
        plain.path().join(".quecto/swarm.sqlite")
    );
    forget_pin(plain.path());
    let newcomer = SwarmContext {
        member: "late".into(),
        ..context.clone()
    };
    assert_eq!(
        newcomer.database(),
        plain.path().join(".git/quecto/swarm.sqlite")
    );
    let refused = super::super::swarm_lifecycle::join_current_process(
        &newcomer,
        None,
        super::super::swarm_bridge::Participation::none(),
        false,
    )
    .unwrap_err()
    .to_string();
    assert!(refused.contains("store missing"), "{refused}");
}

/// A board removed from under a run fails as missing where it was: it is
/// never replaced by another file that happens to exist.
#[test]
fn a_removed_board_fails_as_missing_where_it_was() {
    let repo = repository();
    let (context, _) = created_run(repo.path());
    std::fs::write(repo.path().join(".quecto/swarm.sqlite"), "some other board").unwrap();
    std::fs::remove_dir_all(repo.path().join(".git/quecto")).unwrap();
    let error = context.control_status().unwrap_err().to_string();
    assert!(
        error.contains(&format!(
            "coordination store missing at {}",
            repo.path().join(".git/quecto/swarm.sqlite").display()
        )),
        "{error}"
    );
}

/// In a git checkout the host reads only the board in the git directory: a
/// work-tree `.quecto/swarm.sqlite` (committed, left from before #2145, or
/// planted) is never taken for a run, and a board removed from the git
/// directory is no run, not a fall back to another file.
#[test]
fn the_host_never_reads_a_work_tree_board_in_a_git_checkout() {
    let repo = repository();
    let elsewhere = tempfile::tempdir().unwrap();
    let (board, _) = created_run(elsewhere.path());
    std::fs::copy(board.database(), repo.path().join(".quecto/swarm.sqlite")).unwrap();
    let hosted = super::super::swarm_bridge::HostedStore::at(repo.path().to_path_buf());
    assert!(hosted.hosted_run().unwrap().is_none());
    let (_, id) = created_run(repo.path());
    let run = hosted
        .hosted_run()
        .unwrap()
        .expect("the git directory's board");
    assert_eq!(serde_json::Value::from(run.id).to_string(), id);
    std::fs::remove_dir_all(repo.path().join(".git/quecto")).unwrap();
    assert!(hosted.hosted_run().unwrap().is_none());
}

/// The host opens a store only where it really is inside the checkout: a
/// `.git/quecto` linked elsewhere is refused.
#[test]
fn the_host_refuses_a_store_linked_outside_its_checkout() {
    let repo = repository();
    let elsewhere = tempfile::tempdir().unwrap();
    let (planted, _) = created_run(elsewhere.path());
    std::os::unix::fs::symlink(
        planted.database().parent().unwrap(),
        repo.path().join(".git/quecto"),
    )
    .unwrap();
    let hosted = super::super::swarm_bridge::HostedStore::at(repo.path().to_path_buf());
    let error = hosted.hosted_run().unwrap_err().to_string();
    assert!(error.contains("is outside its checkout"), "{error}");
}

/// A store removed from under a run names its path, not "unavailable or
/// contended".
#[test]
fn a_missing_store_names_its_path() {
    let (_directory, context) = crate::swarm_control_fixture::context();
    std::fs::remove_file(context.database()).unwrap();
    let error = context.control_status().unwrap_err().to_string();
    assert!(
        error.contains(&format!(
            "coordination store missing at {}: it was deleted while the run was live",
            context.database().display()
        )),
        "{error}"
    );
    assert!(!error.contains("contended"), "{error}");
}

/// Where the store should be there is no store file (none yet, or a
/// directory in its place): the host finds no run, and does not try to
/// open one.
#[test]
fn the_host_finds_no_run_where_there_is_no_store_file() {
    let repo = repository();
    let hosted = super::super::swarm_bridge::HostedStore::at(repo.path().to_path_buf());
    assert!(hosted.hosted_run().unwrap().is_none());
    std::fs::create_dir_all(repo.path().join(".quecto/swarm.sqlite")).unwrap();
    assert!(hosted.hosted_run().unwrap().is_none());
}

/// The host pins nothing: once a checkout's layout places a new board (here
/// a checkout that gained a git directory and a new run), the host reads
/// that board, not the one it read before.
#[test]
fn the_host_follows_the_layout_on_every_read() {
    let plain = tempfile::tempdir().unwrap();
    let (_, first) = created_run(plain.path());
    let hosted = super::super::swarm_bridge::HostedStore::at(plain.path().to_path_buf());
    let run = hosted.hosted_run().unwrap().expect("the first board");
    assert_eq!(serde_json::Value::from(run.id).to_string(), first);
    git(plain.path(), &["init", "-q"]);
    // The new board is put in place directly: nothing here touches a pin.
    let elsewhere = tempfile::tempdir().unwrap();
    let (board, second) = created_run(elsewhere.path());
    assert_ne!(first, second);
    std::fs::create_dir_all(plain.path().join(".git/quecto")).unwrap();
    std::fs::copy(
        board.database(),
        plain.path().join(".git/quecto/swarm.sqlite"),
    )
    .unwrap();
    let run = hosted.hosted_run().unwrap().expect("the second board");
    assert_eq!(serde_json::Value::from(run.id).to_string(), second);
}

/// Preparing a checkout for a join makes the store's directory, in the git
/// directory.
#[test]
fn preparing_a_checkout_makes_the_store_directory() {
    let repo = repository();
    super::super::swarm_lifecycle::prepare_checkout(&context(repo.path())).unwrap();
    assert!(repo.path().join(".git/quecto").is_dir());
}

/// PR #2148 review: everything the swarm keeps (the board, and each
/// execution's artifacts beside it) survives every git command agents run,
/// `git clean -fdx` included; the swarm's own files are never reported as
/// files the program wrote.
#[tokio::test]
async fn all_swarm_state_survives_every_git_command_agents_run() {
    let repo = repository();
    let workspace = std::sync::Arc::new(repo.path().to_path_buf());
    let tool = super::super::swarm_test_support::tool(
        workspace.clone(),
        std::sync::Arc::new(crate::infrastructure::security::sandbox::Sandbox::new(
            Some(repo.path().to_path_buf()),
        )),
        super::super::swarm::SwarmConfig::default(),
    );
    use crate::application::tools::ports::Tool;
    let result = tool
        .execute(r#"{"op":"run","code":"open('made.txt','w').write('x'); print('evidence')"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    let answer: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    let execution = answer["execution_id"].as_str().unwrap();
    let artifact_dir = artifact_root(repo.path()).join(execution);
    assert_eq!(
        artifact_dir,
        repo.path().join(".git/quecto/swarm").join(execution)
    );
    let artifacts: Vec<std::path::PathBuf> = std::fs::read_dir(&artifact_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert!(
        artifacts.iter().any(|path| path.ends_with("stdout.txt")),
        "{artifacts:?}"
    );
    let written = answer["files_created_or_modified"].to_string();
    assert!(written.contains("made.txt"), "{answer}");
    assert!(!written.contains(".git/"), "{answer}");
    let context = context(repo.path());
    let id = run_id(&context);
    for command in [
        &["stash", "-u"][..],
        &["clean", "-fdx"],
        &["clean", "-fdX"],
        &["checkout", "-q", "-b", "other"],
        &["checkout", "-q", "main"],
        &["reset", "-q", "--hard"],
    ] {
        git(repo.path(), command);
        assert_eq!(run_id(&context), id, "board after git {command:?}");
        for artifact in &artifacts {
            assert!(artifact.is_file(), "{artifact:?} after git {command:?}");
        }
    }
}
