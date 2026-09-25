//! Where a container's swarm coordination store lives (#2145).
//!
//! The board belongs to the checkout's git directory whenever it has one:
//! `.git/quecto/swarm.sqlite`, or, in a linked worktree (`.git` is a file),
//! that worktree's own git directory. Git's work-tree commands never touch a
//! git directory: an agent's `git stash -u`, `git clean -fdx`, `checkout`,
//! `merge` or `reset --hard` deleted or replaced the board when it lived in
//! the work tree. Only a checkout with no git directory at all keeps it at
//! `.quecto/swarm.sqlite`, where no git command reaches either.
//!
//! The location follows from the checkout's layout alone, never from which
//! files happen to exist, so a stale board an agent once committed at the old
//! path is never adopted. A member pins the path once it has found its board:
//! a layout that changes mid-run (a `git init` in a checkout that had none, a
//! git directory created or removed) never moves a live board; a board that
//! disappears fails as missing, and a member joining after such a change finds
//! no store and is refused. The host pins nothing: each read follows the
//! layout, and one that finds no board sees no run.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The store's directory inside a git directory.
const GIT_STORE_DIR: &str = "quecto";
/// The store's directory in a checkout with no git directory.
const WORK_TREE_STORE_DIR: &str = ".quecto";
const STORE_FILE: &str = "swarm.sqlite";

/// The artifacts' directory beside the board.
const ARTIFACT_DIR: &str = "swarm";

/// Boards this member process has found, by canonical checkout: never
/// re-decided.
static PINNED: Mutex<BTreeMap<PathBuf, PathBuf>> = Mutex::new(BTreeMap::new());

/// Where the store of the container checked out at `checkout` lives, by the
/// checkout's layout.
pub(super) fn located(checkout: &Path) -> PathBuf {
    match own_git_dir(checkout) {
        Some(git_dir) => git_dir.join(GIT_STORE_DIR).join(STORE_FILE),
        None => checkout.join(WORK_TREE_STORE_DIR).join(STORE_FILE),
    }
}

/// A member's coordination store in the container checked out at
/// `checkout`: the board it found there, or where it will be.
pub(super) fn member_store_path(checkout: &Path) -> PathBuf {
    let key = pin_key(checkout);
    let mut pinned = PINNED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(board) = pinned.get(&key) {
        return board.clone();
    }
    let board = located(checkout);
    if board.is_file() {
        pinned.insert(key, board.clone());
    }
    board
}

/// One key per checkout however it is spelled.
fn pin_key(checkout: &Path) -> PathBuf {
    std::fs::canonicalize(checkout).unwrap_or_else(|_| checkout.to_path_buf())
}

/// The git directory of the repository or worktree rooted at `checkout`:
/// `.git` itself, or the directory a `.git` file names. `None` when there is
/// none (no repository, or a pointer to a directory that is not there).
fn own_git_dir(checkout: &Path) -> Option<PathBuf> {
    let dot_git = checkout.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let pointer = std::fs::read_to_string(&dot_git).ok()?;
    let named = pointer.trim().strip_prefix("gitdir:")?.trim();
    let git_dir = checkout.join(named);
    git_dir.is_dir().then_some(git_dir)
}

#[cfg(test)]
/// As a new process would: forget the board found at `checkout`.
pub(super) fn forget_pin(checkout: &Path) {
    PINNED
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&pin_key(checkout));
}

/// The directory, beside the board, where the swarm tool keeps each
/// execution's artifacts (`<execution>/stdout.txt`, `stderr.txt`): out of
/// git's reach like the board, so a live run's evidence survives
/// `git clean -fdx`.
pub(super) fn artifact_root(workspace: &Path) -> PathBuf {
    board_dir(workspace).join(ARTIFACT_DIR)
}

/// Whether `path` is the swarm's own state rather than a file a program
/// wrote: the board, its journal files, or the artifacts.
pub(super) fn is_swarm_state(workspace: &Path, path: &Path) -> bool {
    let board = member_store_path(workspace);
    let journal = |name: &std::ffi::OsStr| {
        board
            .file_name()
            .and_then(|board| name.to_str().zip(board.to_str()))
            .is_some_and(|(name, board)| name.starts_with(board))
    };
    let beside_board = path.parent() == board.parent();
    path.starts_with(artifact_root(workspace))
        || (beside_board && path.file_name().is_some_and(journal))
}

fn board_dir(workspace: &Path) -> PathBuf {
    member_store_path(workspace)
        .parent()
        .expect("the store has a directory")
        .to_path_buf()
}

#[cfg(test)]
#[path = "swarm_store_location_tests.rs"]
mod tests;
