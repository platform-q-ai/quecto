use super::*;
use std::path::Path;
use std::process::Command;

/// git, isolated from the user's configuration.
fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(["-c", "user.email=t@t", "-c", "user.name=t"])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HOME", dir)
        .output()
        .expect("git runs");
    assert!(output.status.success(), "git {args:?}: {output:?}");
    String::from_utf8(output.stdout).unwrap()
}

/// A repository shaped like one with a committed standard container: its
/// `.quecto/` is tracked, so the store would be an untracked file in it.
fn repository() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q"]);
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

fn with_store(root: &Path) {
    std::fs::write(root.join(".quecto/swarm.sqlite"), "board").unwrap();
    std::fs::write(root.join(".quecto/swarm.sqlite-journal"), "journal").unwrap();
    std::fs::create_dir_all(root.join(".quecto/swarm/exec-1")).unwrap();
    std::fs::write(root.join(".quecto/swarm/exec-1/out.txt"), "artifact").unwrap();
}

/// #2145: once excluded, the git commands agents run on a checkout leave
/// the store, its journal and swarm artifacts where they are, and never
/// stage them.
#[test]
fn common_git_commands_leave_an_excluded_store_alone() {
    let repo = repository();
    assert_eq!(exclude_from_git(repo.path()), Ok(Excluded::Added));
    with_store(repo.path());
    std::fs::write(repo.path().join("app.py"), "changed\n").unwrap();
    git(repo.path(), &["add", "-A"]);
    assert_eq!(
        git(repo.path(), &["diff", "--cached", "--name-only"]),
        "app.py\n"
    );
    git(repo.path(), &["stash", "-u"]);
    git(repo.path(), &["clean", "-fd"]);
    for kept in [
        ".quecto/swarm.sqlite",
        ".quecto/swarm.sqlite-journal",
        ".quecto/swarm/exec-1/out.txt",
    ] {
        assert!(repo.path().join(kept).exists(), "{kept} was removed");
    }
}

/// Without the exclusion the same commands remove the store: the failure
/// the exclusion prevents.
#[test]
fn without_the_exclusion_git_clean_removes_the_store() {
    let repo = repository();
    with_store(repo.path());
    git(repo.path(), &["clean", "-fd"]);
    assert!(!repo.path().join(".quecto/swarm.sqlite").exists());
}

/// Every join excludes again: nothing is added twice, and what the
/// repository's own exclude file held is kept.
#[test]
fn excluding_again_adds_nothing_and_keeps_existing_entries() {
    let repo = repository();
    let exclude = repo.path().join(".git/info/exclude");
    std::fs::write(&exclude, "# mine\n*.log").unwrap();
    assert_eq!(exclude_from_git(repo.path()), Ok(Excluded::Added));
    assert_eq!(exclude_from_git(repo.path()), Ok(Excluded::AlreadyExcluded));
    let text = std::fs::read_to_string(&exclude).unwrap();
    assert!(text.starts_with("# mine\n*.log\n"), "{text}");
    for entry in STORE_ENTRIES {
        assert_eq!(
            text.lines().filter(|line| *line == entry).count(),
            1,
            "{entry}: {text}"
        );
    }
}

/// A checkout that is a linked worktree keeps its exclusions in the shared
/// git directory, where git reads them for every worktree.
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
    assert_eq!(exclude_from_git(&checkout), Ok(Excluded::Added));
    let shared = std::fs::read_to_string(repo.path().join(".git/info/exclude")).unwrap();
    assert!(shared.contains("/.quecto/swarm.sqlite\n"), "{shared}");
    with_store(&checkout);
    git(&checkout, &["clean", "-fd"]);
    assert!(checkout.join(".quecto/swarm.sqlite").exists());
}

/// A checkout that is not the root of a repository gets no entries: they
/// are anchored at a repository root.
#[test]
fn a_checkout_that_is_no_repository_root_is_left_alone() {
    let plain = tempfile::tempdir().unwrap();
    assert_eq!(
        exclude_from_git(plain.path()),
        Ok(Excluded::NotARepositoryRoot)
    );
    let repo = repository();
    let nested = repo.path().join("sub");
    std::fs::create_dir(&nested).unwrap();
    assert_eq!(exclude_from_git(&nested), Ok(Excluded::NotARepositoryRoot));
    assert!(
        !std::fs::read_to_string(repo.path().join(".git/info/exclude"))
            .unwrap_or_default()
            .contains("swarm.sqlite")
    );
}

/// A `.git` file that names no git directory is reported, not guessed at.
#[test]
fn a_git_file_naming_no_directory_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".git"), "not a pointer\n").unwrap();
    assert!(exclude_from_git(dir.path()).unwrap_err().contains("gitdir"));
}

/// Joining a container's store (what every container agent does at start)
/// excludes it from the checkout's git.
#[test]
fn joining_a_container_store_excludes_it_from_git() {
    let (directory, context) = crate::swarm_control_fixture::context();
    git(directory.path(), &["init", "-q"]);
    super::super::swarm_lifecycle::join_current_process(
        &context,
        None,
        super::super::swarm_bridge::Participation::none(),
    )
    .unwrap();
    let text = std::fs::read_to_string(directory.path().join(".git/info/exclude")).unwrap();
    for entry in STORE_ENTRIES {
        assert!(text.lines().any(|line| line == entry), "{entry}: {text}");
    }
}

/// A store removed from under a run names its path and the likely cause,
/// not "unavailable or contended".
#[test]
fn a_missing_store_names_its_path_and_the_likely_cause() {
    let (_directory, context) = crate::swarm_control_fixture::context();
    std::fs::remove_file(context.database()).unwrap();
    let error = context.control_status().unwrap_err().to_string();
    assert!(
        error.contains(&format!(
            "coordination store missing at {}",
            context.database().display()
        )),
        "{error}"
    );
    assert!(error.contains("git clean"), "{error}");
    assert!(!error.contains("contended"), "{error}");
}
