//! Contract for the `MainWorktreeLocator` port: a linked git worktree's
//! root names its main worktree (absolute, an existing directory); a main
//! worktree, a plain directory and a subdirectory are not linked; a
//! pointer that does not resolve along git's layout — broken, copied
//! elsewhere, or into a bare repository — is unresolved with a reason, so
//! nothing is shared. Real git repositories, never a running git in the
//! adapter.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use quecto::application::configuration::ports::{MainWorktreeLocator, WorktreeLink};
use quecto::infrastructure::config::main_worktree::GitMetadataWorktreeLocator;
use tempfile::TempDir;

fn under_test() -> Arc<dyn MainWorktreeLocator> {
    Arc::new(GitMetadataWorktreeLocator)
}

/// git, isolated from the user's configuration and from any repository the
/// test itself runs inside.
fn git(dir: &Path, args: &[&str]) {
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
}

struct Layout {
    tmp: TempDir,
    main: PathBuf,
    linked: PathBuf,
}

/// `<tmp>/main` with one commit and a linked worktree at
/// `<tmp>/.pocket-worktrees/main/task`.
fn layout() -> Layout {
    let tmp = tempfile::tempdir().unwrap();
    let main = tmp.path().join("main");
    std::fs::create_dir(&main).unwrap();
    git(&main, &["init", "-q", "-b", "master"]);
    std::fs::write(main.join("README"), "x\n").unwrap();
    git(&main, &["add", "-A"]);
    git(&main, &["commit", "-q", "-m", "base"]);
    let linked = tmp.path().join(".pocket-worktrees/main/task");
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "pocket/task",
            linked.to_str().unwrap(),
        ],
    );
    let main = main.canonicalize().unwrap();
    let linked = linked.canonicalize().unwrap();
    Layout { tmp, main, linked }
}

fn locate(directory: &Path) -> WorktreeLink {
    under_test().locate(directory)
}

fn unresolved(directory: &Path, expected: &str) {
    match locate(directory) {
        WorktreeLink::Unresolved { reason } => {
            assert!(reason.contains(expected), "{reason:?} names {expected:?}")
        }
        other => panic!("expected unresolved ({expected}), got {other:?}"),
    }
}

#[test]
fn a_linked_worktree_root_names_its_main_worktree() {
    let layout = layout();
    assert_eq!(
        locate(&layout.linked),
        WorktreeLink::Linked {
            main_worktree: layout.main.clone()
        }
    );
}

#[test]
fn a_main_worktree_a_plain_directory_and_a_subdirectory_are_not_linked() {
    let layout = layout();
    let sub = layout.linked.join("sub");
    std::fs::create_dir(&sub).unwrap();
    assert_eq!(locate(&layout.main), WorktreeLink::NotLinked);
    assert_eq!(locate(&sub), WorktreeLink::NotLinked);
    assert_eq!(locate(layout.tmp.path()), WorktreeLink::NotLinked);
}

#[test]
fn a_gitdir_pointer_that_does_not_resolve_is_unresolved() {
    let layout = layout();
    std::fs::write(layout.linked.join(".git"), "gitdir: /nonexistent/x\n").unwrap();
    unresolved(&layout.linked, "/nonexistent/x");
    std::fs::write(layout.linked.join(".git"), "not a pointer\n").unwrap();
    unresolved(&layout.linked, "is not a gitdir pointer");
}

/// A `.git` file copied elsewhere (an unpacked archive, say) names a real
/// worktree's git directory, which does not name it back: that directory
/// never shares the repository's overlay.
#[test]
fn a_pointer_whose_git_directory_does_not_name_it_back_is_unresolved() {
    let layout = layout();
    let impostor = layout.tmp.path().join("impostor");
    std::fs::create_dir(&impostor).unwrap();
    std::fs::copy(layout.linked.join(".git"), impostor.join(".git")).unwrap();
    unresolved(&impostor, "does not name");
}

#[test]
fn a_worktree_of_a_bare_repository_is_unresolved() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source");
    std::fs::create_dir(&source).unwrap();
    git(&source, &["init", "-q", "-b", "master"]);
    std::fs::write(source.join("README"), "x\n").unwrap();
    git(&source, &["add", "-A"]);
    git(&source, &["commit", "-q", "-m", "base"]);
    git(tmp.path(), &["clone", "-q", "--bare", "source", "bare.git"]);
    git(
        &tmp.path().join("bare.git"),
        &["worktree", "add", "-q", "../wt"],
    );
    unresolved(&tmp.path().join("wt"), "not a main worktree's .git");
    // The same bare repository kept in a `.git` directory (the dotfiles
    // layout) has no main worktree either.
    git(tmp.path(), &["clone", "-q", "--bare", "source", "dot/.git"]);
    git(
        &tmp.path().join("dot/.git"),
        &["worktree", "add", "-q", "../../wt2"],
    );
    unresolved(&tmp.path().join("wt2"), "core.bare");
}

/// A working directory that no longer exists is not a linked worktree:
/// there is nothing to resolve, so nothing is reported (not a diagnostic).
#[test]
fn a_deleted_working_directory_is_not_linked() {
    let tmp = tempfile::tempdir().unwrap();
    let gone = tmp.path().join("gone");
    std::fs::create_dir(&gone).unwrap();
    std::fs::remove_dir(&gone).unwrap();
    assert_eq!(locate(&gone), WorktreeLink::NotLinked);
}
