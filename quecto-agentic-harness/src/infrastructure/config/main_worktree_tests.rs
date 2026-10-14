use super::*;
use std::process::Command;
use tempfile::TempDir;

/// git, isolated from the user's configuration and from any repository the
/// test itself runs inside (a hook's GIT_DIR and friends).
pub(crate) fn git(dir: &Path, args: &[&str]) {
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

/// A repository at `<tmp>/main` with one commit and a linked worktree at
/// `<tmp>/.pocket-worktrees/main/task` on `pocket/task`, as the Pocket
/// relay lays them out.
pub(crate) struct Layout {
    pub tmp: TempDir,
    pub main: PathBuf,
    pub linked: PathBuf,
}

pub(crate) fn layout() -> Layout {
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
    GitMetadataWorktreeLocator.locate(directory)
}

fn unresolved(directory: &Path, expected: &str) {
    match locate(directory) {
        WorktreeLink::Unresolved { reason } => {
            assert!(reason.contains(expected), "{reason:?} names {expected:?}")
        }
        other => panic!("expected unresolved ({expected}), got {other:?}"),
    }
}

/// Git may record both pointers relative (`worktree.useRelativePaths`).
#[test]
fn relative_pointers_resolve_to_the_same_main_worktree() {
    let layout = layout();
    let admin = layout.main.join(".git/worktrees/task");
    std::fs::write(
        layout.linked.join(".git"),
        "gitdir: ../../../main/.git/worktrees/task\n",
    )
    .unwrap();
    std::fs::write(
        admin.join("gitdir"),
        "../../../../.pocket-worktrees/main/task/.git\n",
    )
    .unwrap();
    assert_eq!(
        locate(&layout.linked),
        WorktreeLink::Linked {
            main_worktree: layout.main.clone()
        }
    );
}

#[test]
fn a_symlinked_dot_git_is_unresolved() {
    let layout = layout();
    let linked_dir = layout.tmp.path().join("linkdir");
    std::fs::create_dir(&linked_dir).unwrap();
    std::os::unix::fs::symlink(layout.linked.join(".git"), linked_dir.join(".git")).unwrap();
    unresolved(&linked_dir, "neither a directory nor a gitdir file");
}

#[test]
fn a_relocated_main_work_tree_is_unresolved() {
    let layout = layout();
    git(&layout.main, &["config", "core.worktree", "/elsewhere"]);
    unresolved(&layout.linked, "core.worktree");
}

/// A submodule's (or `--separate-git-dir`) `.git` file names a git
/// directory that is its own common directory: not a linked worktree.
#[test]
fn a_separate_git_directory_is_not_linked() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path().join("work");
    std::fs::create_dir(&work).unwrap();
    let separate = tmp.path().join("store.git");
    git(
        &work,
        &[
            "init",
            "-q",
            "--separate-git-dir",
            separate.to_str().unwrap(),
        ],
    );
    assert_eq!(locate(&work), WorktreeLink::NotLinked);
}

#[test]
fn core_layout_accepts_only_an_explicitly_non_bare_unrelocated_repository() {
    let ok = "[core]\n\trepositoryformatversion = 0\n\tbare = false\n\tlogallrefupdates = true\n";
    assert_eq!(core_layout(ok, true), Ok(()));
    assert_eq!(core_layout("[core]\n\tBare=No ; c\n", true), Ok(()));
    assert_eq!(
        core_layout(
            "[remote \"o\"]\n\tbare = true\n[core]\nbare = false\n",
            true
        ),
        Ok(())
    );
    for refused in [
        "[core]\n\tbare = true\n",
        "[core]\n\tbare\n",
        "[core]\n\tbare = maybe\n",
        "[core]\n\trepositoryformatversion = 0\n",
        "[core]\nbare = false\n\tworktree = /x\n",
        "[core]\nbare = false\n[include]\n\tpath = other\n",
        "[core]\nbare = false\n[includeIf \"gitdir:x\"]\n\tpath = other\n",
        "[core] bare = true\n",
    ] {
        assert!(
            core_layout(refused, true).is_err(),
            "{refused:?} is refused"
        );
    }
    // The per-worktree file may leave `bare` unset, never relocate.
    assert_eq!(core_layout("", false), Ok(()));
    assert!(core_layout("[core]\n\tworktree = /x\n", false).is_err());
}

/// A pointer naming a directory whose metadata is not regular files (a
/// FIFO would block the read) is refused before anything is opened.
#[test]
fn metadata_that_is_not_a_regular_file_is_unresolved() {
    let layout = layout();
    let admin = layout.main.join(".git/worktrees/task");
    std::fs::remove_file(admin.join("commondir")).unwrap();
    std::os::unix::fs::symlink("../..", admin.join("commondir")).unwrap();
    unresolved(&layout.linked, "is not a regular file");
    std::fs::remove_file(admin.join("commondir")).unwrap();
    let status = Command::new("mkfifo")
        .arg(admin.join("commondir"))
        .status()
        .unwrap();
    assert!(status.success());
    unresolved(&layout.linked, "is not a regular file");
}
