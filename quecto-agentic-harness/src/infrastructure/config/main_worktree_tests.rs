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
        "[core] worktree=/x\nbare = false\n",
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

/// Review round 1 M1: an admin directory fabricated outside the common
/// directory's `worktrees/` (pointer, back-link and `commondir` all
/// written by whoever controls the checkout) names a trusted repository's
/// `.git` as its common directory. Git never registered it, so nothing of
/// that repository is borrowed.
#[test]
fn an_admin_directory_git_never_registered_is_unresolved() {
    let layout = layout();
    let fake = layout.tmp.path().join("fake");
    let admin = fake.join("adm");
    std::fs::create_dir_all(&admin).unwrap();
    std::fs::write(fake.join(".git"), "gitdir: ./adm\n").unwrap();
    std::fs::write(admin.join("gitdir"), "../.git\n").unwrap();
    std::fs::write(
        admin.join("commondir"),
        format!("{}\n", layout.main.join(".git").display()),
    )
    .unwrap();
    unresolved(&fake, "is not registered in");
    // Relative, as in the review's reproduction.
    std::fs::write(admin.join("commondir"), "../../main/.git\n").unwrap();
    unresolved(&fake, "is not registered in");
}

/// Review round 1 L1: a metadata file swapped for a symbolic link or a
/// FIFO is refused by the open itself (no follow, no blocking), not by a
/// check that a swap could race.
#[test]
fn a_pointer_swapped_for_a_link_or_a_fifo_is_refused_without_blocking() {
    let layout = layout();
    let admin = layout.main.join(".git/worktrees/task");
    let real = admin.join("gitdir.real");
    std::fs::rename(admin.join("gitdir"), &real).unwrap();
    std::os::unix::fs::symlink(&real, admin.join("gitdir")).unwrap();
    unresolved(&layout.linked, "gitdir");
    std::fs::remove_file(admin.join("gitdir")).unwrap();
    let status = Command::new("mkfifo")
        .arg(admin.join("gitdir"))
        .status()
        .unwrap();
    assert!(status.success());
    unresolved(&layout.linked, "is not a regular file");
}

/// The metadata limit is inclusive: a pointer padded to exactly 64 KiB is
/// read; one byte more is not git's.
#[test]
fn a_metadata_file_at_the_limit_is_read_and_one_byte_over_is_refused() {
    let layout = layout();
    let pointer = std::fs::read_to_string(layout.linked.join(".git")).unwrap();
    let padded = |len: usize| {
        let mut text = pointer.clone();
        text.push_str(&"\n".repeat(len - pointer.len()));
        assert_eq!(text.len(), len);
        text
    };
    std::fs::write(layout.linked.join(".git"), padded(64 * 1024)).unwrap();
    assert_eq!(
        locate(&layout.linked),
        WorktreeLink::Linked {
            main_worktree: layout.main.clone()
        }
    );
    std::fs::write(layout.linked.join(".git"), padded(64 * 1024 + 1)).unwrap();
    unresolved(&layout.linked, "too large");
}

/// Review round 2 MED-1: the `.git` pointer read is the one the back-link
/// is checked against. A looping attacker who swaps their checkout's
/// `.git` for a link to a registered worktree's `.git` right after the
/// read must not have the link followed: the back-link is compared with
/// `<canonical directory>/.git`, never with a canonicalised `.git`.
#[test]
fn a_dot_git_swapped_for_a_link_after_the_read_is_refused() {
    let layout = layout();
    let attacker = layout.tmp.path().join("attacker");
    std::fs::create_dir(&attacker).unwrap();
    std::fs::copy(layout.linked.join(".git"), attacker.join(".git")).unwrap();
    let victim = layout.linked.join(".git");
    let mut swap = || {
        std::fs::remove_file(attacker.join(".git")).unwrap();
        std::os::unix::fs::symlink(&victim, attacker.join(".git")).unwrap();
    };
    let reason = main_worktree_with_seam(&attacker, &mut swap).unwrap_err();
    assert!(reason.contains("does not name"), "{reason}");
}

/// Review round 2 MED-1: the back-link must name the very file read — the
/// same (device, inode) — not merely the same path, so a pointer swapped
/// for another file after the read is refused.
#[test]
fn a_dot_git_replaced_after_the_read_is_refused() {
    let layout = layout();
    let dot_git = layout.linked.join(".git");
    let mut replace = || {
        let content = std::fs::read(&dot_git).unwrap();
        let fresh = layout.tmp.path().join("fresh");
        std::fs::write(&fresh, content).unwrap();
        std::fs::rename(&fresh, &dot_git).unwrap();
    };
    let reason = main_worktree_with_seam(&layout.linked, &mut replace).unwrap_err();
    assert!(reason.contains("is not the file read"), "{reason}");
    // Without a swap the same worktree resolves.
    assert_eq!(
        main_worktree_with_seam(&layout.linked, &mut || {}),
        Ok(Some(layout.main.clone()))
    );
}

/// Review round 2 LOW-3: a back-link naming a worktree that no longer
/// exists (it cannot be canonicalised) names nobody.
#[test]
fn a_back_link_to_a_deleted_worktree_is_refused() {
    let layout = layout();
    let admin = layout.main.join(".git/worktrees/task");
    let gone = layout.tmp.path().join("deleted/.git");
    std::fs::write(admin.join("gitdir"), format!("{}\n", gone.display())).unwrap();
    unresolved(&layout.linked, "does not name");
}

/// Review round 2 LOW-1: parity with git. A worktree deleted without
/// `git worktree prune` stays registered; recreating its path with the
/// same pointer is accepted, as git itself accepts it.
#[test]
fn a_recreated_path_of_an_unpruned_worktree_is_accepted_as_git_accepts_it() {
    let layout = layout();
    let pointer = std::fs::read(layout.linked.join(".git")).unwrap();
    std::fs::remove_dir_all(&layout.linked).unwrap();
    std::fs::create_dir_all(&layout.linked).unwrap();
    std::fs::write(layout.linked.join(".git"), pointer).unwrap();
    assert_eq!(
        locate(&layout.linked),
        WorktreeLink::Linked {
            main_worktree: layout.main.clone()
        }
    );
}

#[test]
fn a_pointer_or_commondir_holding_more_than_one_line_is_refused() {
    let layout = layout();
    let pointer = std::fs::read_to_string(layout.linked.join(".git")).unwrap();
    std::fs::write(
        layout.linked.join(".git"),
        format!("{pointer}gitdir: /elsewhere\n"),
    )
    .unwrap();
    unresolved(&layout.linked, "is not a gitdir pointer");
    std::fs::write(layout.linked.join(".git"), pointer).unwrap();
    let commondir = layout.main.join(".git/worktrees/task/commondir");
    std::fs::write(&commondir, "../..\n../../x\n").unwrap();
    unresolved(&layout.linked, "does not hold one path");
}

/// The identity compared is the file's device and inode, both taken from
/// the metadata: a file on another device with the same inode is another
/// file.
#[test]
fn a_file_identity_is_its_device_and_inode() {
    use std::os::unix::fs::MetadataExt;
    let layout = layout();
    let entry = std::fs::symlink_metadata(layout.linked.join(".git")).unwrap();
    let identity = FileIdentity::of(&entry);
    assert_eq!(
        identity,
        FileIdentity {
            device: entry.dev(),
            inode: entry.ino()
        }
    );
    assert_ne!(
        identity,
        FileIdentity {
            device: entry.dev() + 1,
            inode: entry.ino()
        }
    );
}
