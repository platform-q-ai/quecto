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

/// Create a run as the container's creator does: claim, then create.
fn created_run(checkout: &Path) -> (SwarmContext, String) {
    let context = context(checkout);
    claim(checkout, true).unwrap();
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
    let id = context.control_status().unwrap()["id"].to_string();
    (context, id)
}

/// The store lives in the git directory only once the creator claimed it
/// there; anyone else, or a checkout with no repository, leaves it in the
/// work tree.
#[test]
fn only_the_creator_claims_the_git_directory_for_the_store() {
    let repo = repository();
    let work_tree = repo.path().join(".quecto/swarm.sqlite");
    assert_eq!(store_path(repo.path()), work_tree);
    claim(repo.path(), false).unwrap();
    assert_eq!(store_path(repo.path()), work_tree);
    claim(repo.path(), true).unwrap();
    assert_eq!(
        store_path(repo.path()),
        repo.path().join(".git/quecto/swarm.sqlite")
    );
    let plain = tempfile::tempdir().unwrap();
    claim(plain.path(), true).unwrap();
    assert_eq!(
        store_path(plain.path()),
        plain.path().join(".quecto/swarm.sqlite")
    );
}

/// #2145: with the store in the git directory, the git commands agents run
/// on a checkout neither delete nor replace the live board, even where a
/// branch tracks a stale `.quecto/swarm.sqlite`.
#[test]
fn a_claimed_store_survives_every_git_command_agents_run() {
    let repo = repository();
    // A branch where an agent once committed a board.
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
        assert_eq!(
            status.unwrap()["id"].to_string(),
            id,
            "after git {command:?}"
        );
    }
}

/// A creator never claims the git directory over a board already in the
/// work tree: a run started before this layout keeps its board.
#[test]
fn a_creator_never_moves_a_board_already_in_the_work_tree() {
    let repo = repository();
    std::fs::write(repo.path().join(".quecto/swarm.sqlite"), "live board").unwrap();
    claim(repo.path(), true).unwrap();
    assert!(!repo.path().join(".git/quecto").exists());
    assert_eq!(
        store_path(repo.path()),
        repo.path().join(".quecto/swarm.sqlite")
    );
}

/// When git cannot say whether a work-tree board is committed (here, a
/// corrupt index), the creator claims the git directory: a stale committed
/// board is the case that happens, and it is never adopted.
#[test]
fn a_creator_claims_when_git_cannot_tell_a_board_is_committed() {
    let repo = repository();
    std::fs::write(repo.path().join(".quecto/swarm.sqlite"), "board").unwrap();
    std::fs::write(repo.path().join(".git/index"), "not an index").unwrap();
    claim(repo.path(), true).unwrap();
    assert_eq!(
        store_path(repo.path()),
        repo.path().join(".git/quecto/swarm.sqlite")
    );
}

/// A board left in the work tree (nothing claimed the git directory) is
/// excluded: `git stash -u` and `git clean -fd` leave it, `git add -A`
/// never stages it.
#[test]
fn a_board_left_in_the_work_tree_is_excluded_from_git() {
    let repo = repository();
    assert_eq!(exclude_work_tree(repo.path()), Ok(Exclusion::Added));
    std::fs::write(repo.path().join(".quecto/swarm.sqlite"), "board").unwrap();
    std::fs::write(repo.path().join(".quecto/swarm.sqlite-journal"), "journal").unwrap();
    std::fs::write(repo.path().join("app.py"), "changed\n").unwrap();
    git(repo.path(), &["add", "-A"]);
    assert_eq!(
        git(repo.path(), &["diff", "--cached", "--name-only"]),
        "app.py\n"
    );
    git(repo.path(), &["stash", "-u"]);
    git(repo.path(), &["clean", "-fd"]);
    assert!(repo.path().join(".quecto/swarm.sqlite").exists());
    assert!(repo.path().join(".quecto/swarm.sqlite-journal").exists());
}

/// The host opens a store only where it really is inside the checkout: a
/// `.git/quecto` linked elsewhere is refused.
#[test]
fn the_host_refuses_a_store_linked_outside_its_checkout() {
    let repo = repository();
    let elsewhere = tempfile::tempdir().unwrap();
    let (planted, _) = created_run(elsewhere.path());
    assert!(planted.database().is_file());
    std::os::unix::fs::symlink(
        planted.database().parent().unwrap(),
        repo.path().join(".git/quecto"),
    )
    .unwrap();
    let hosted = super::super::swarm_bridge::HostedStore::at(repo.path().to_path_buf());
    let error = hosted.hosted_run().unwrap_err().to_string();
    assert!(error.contains("is outside its checkout"), "{error}");
}

/// A fresh clone of a repository that tracks a stale board: the creator
/// claims the git directory first, so the stale board is never adopted.
#[test]
fn a_stale_board_committed_in_the_repository_is_never_adopted() {
    let repo = repository();
    std::fs::write(repo.path().join(".quecto/swarm.sqlite"), "stale board").unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-q", "-m", "committed a board"]);
    claim(repo.path(), true).unwrap();
    let context = context(repo.path());
    assert!(!context.run_created().unwrap());
}

/// A repository initialised mid-run in a checkout that had none does not
/// move the live store.
#[test]
fn a_repository_initialised_mid_run_leaves_the_store_where_it_is() {
    let plain = tempfile::tempdir().unwrap();
    let (context, id) = created_run(plain.path());
    git(plain.path(), &["init", "-q"]);
    assert_eq!(
        context.database(),
        plain.path().join(".quecto/swarm.sqlite")
    );
    assert_eq!(context.control_status().unwrap()["id"].to_string(), id);
}

/// Swarm artifacts are excluded: `git add -A` never stages them and
/// `git stash -u` / `git clean -fd` leave them.
#[test]
fn swarm_artifacts_are_left_alone_by_git() {
    let repo = repository();
    assert_eq!(exclude_work_tree(repo.path()), Ok(Exclusion::Added));
    std::fs::create_dir_all(repo.path().join(".quecto/swarm/exec-1")).unwrap();
    std::fs::write(repo.path().join(".quecto/swarm/exec-1/out.txt"), "artifact").unwrap();
    std::fs::write(repo.path().join("app.py"), "changed\n").unwrap();
    git(repo.path(), &["add", "-A"]);
    assert_eq!(
        git(repo.path(), &["diff", "--cached", "--name-only"]),
        "app.py\n"
    );
    git(repo.path(), &["stash", "-u"]);
    git(repo.path(), &["clean", "-fd"]);
    assert!(repo.path().join(".quecto/swarm/exec-1/out.txt").exists());
}

/// Excluding again adds nothing, and what the exclude file held is kept.
#[test]
fn excluding_again_adds_nothing_and_keeps_existing_entries() {
    let repo = repository();
    let exclude = repo.path().join(".git/info/exclude");
    std::fs::write(&exclude, "# mine\n*.log").unwrap();
    assert_eq!(exclude_work_tree(repo.path()), Ok(Exclusion::Added));
    assert_eq!(
        exclude_work_tree(repo.path()),
        Ok(Exclusion::AlreadyPresent)
    );
    let text = std::fs::read_to_string(&exclude).unwrap();
    assert!(text.starts_with("# mine\n*.log\n"), "{text}");
    for entry in WORK_TREE_ENTRIES {
        assert_eq!(
            text.lines().filter(|line| *line == entry).count(),
            1,
            "{entry}: {text}"
        );
    }
    let leftovers: Vec<_> = std::fs::read_dir(repo.path().join(".git/info"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, ["exclude"], "nothing is left aside");
}

/// A linked worktree keeps its exclusions in the shared git directory.
#[test]
fn a_worktree_checkout_is_excluded_in_the_shared_git_directory() {
    let repo = repository();
    let worktree = tempfile::tempdir().unwrap();
    let checkout = worktree.path().join("wt");
    git(
        repo.path(),
        &["worktree", "add", "-q", checkout.to_str().unwrap()],
    );
    assert!(checkout.join(".git").is_file());
    assert_eq!(exclude_work_tree(&checkout), Ok(Exclusion::Added));
    let shared = std::fs::read_to_string(repo.path().join(".git/info/exclude")).unwrap();
    assert!(shared.contains("/.quecto/swarm/\n"), "{shared}");
}

/// No repository root, no entries; a `.git` file naming nothing is an error.
#[test]
fn a_checkout_that_is_no_repository_root_is_left_alone() {
    let plain = tempfile::tempdir().unwrap();
    assert_eq!(
        exclude_work_tree(plain.path()),
        Ok(Exclusion::NotARepositoryRoot)
    );
    let repo = repository();
    let nested = repo.path().join("sub");
    std::fs::create_dir(&nested).unwrap();
    assert_eq!(
        exclude_work_tree(&nested),
        Ok(Exclusion::NotARepositoryRoot)
    );
    let broken = tempfile::tempdir().unwrap();
    std::fs::write(broken.path().join(".git"), "not a pointer\n").unwrap();
    assert!(
        exclude_work_tree(broken.path())
            .unwrap_err()
            .contains("gitdir")
    );
}

/// Preparing a checkout for a join makes the store's directory (here, one
/// nobody claimed: the work tree's) and excludes the artifacts.
#[test]
fn preparing_a_checkout_makes_the_store_directory_and_excludes_artifacts() {
    let repo = repository();
    super::super::swarm_lifecycle::prepare_checkout(&context(repo.path())).unwrap();
    assert!(
        repo.path().join(".quecto").is_dir(),
        "the unclaimed store's directory"
    );
    let text = std::fs::read_to_string(repo.path().join(".git/info/exclude")).unwrap();
    assert!(text.contains("/.quecto/swarm/\n"), "{text}");
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
