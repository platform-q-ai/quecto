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

/// What the host finds at every place a checkout's board may be (#2206).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum BoardPresence {
    /// Nothing at any of them: each place answered "no such file".
    Absent,
    /// Something where the layout places the board now — and, when
    /// `displaced` is set, also at a place it no longer names (#2206 round
    /// 3): a live run a member pinned there is hidden unless the current
    /// board holds a created run of its own.
    Current {
        board: PathBuf,
        displaced: Option<PathBuf>,
    },
    /// Something only where the layout does NOT place it now: the work
    /// tree's `.quecto/swarm.sqlite` once the checkout has a git directory
    /// (an agent's `git init` in a sandbox with no clone), or the git
    /// directory's board when the layout now says work tree. A member
    /// pinned it and may still be running on it.
    Displaced(PathBuf),
    /// A place could not be examined (permissions, a symlink loop, I/O, a
    /// `.git` pointer that does not resolve on this host): never proof
    /// that there is no board.
    Unknown(String),
}

/// Look at every place a checkout's board may be — where its layout places
/// it now, the work tree's `.quecto/swarm.sqlite` and the git directory's
/// `.git/quecto/swarm.sqlite` — without following a link. Only a place
/// that answers "no such file" counts as empty, and "not a directory" only
/// for `.git/quecto/…` when `.git` is a worktree pointer that resolves on
/// this host (its board then lives in the git directory it names). Any
/// other answer is `Unknown` (#2206 rounds 2 and 3).
pub(super) fn board_presence(checkout: &Path) -> BoardPresence {
    let dot_git = checkout.join(".git");
    let pointer_resolved = match std::fs::symlink_metadata(&dot_git) {
        Ok(meta) if meta.is_file() => match own_git_dir(checkout) {
            Some(_) => true,
            None => {
                return BoardPresence::Unknown(format!(
                    "{} names no git directory that resolves on this host",
                    dot_git.display()
                ));
            }
        },
        Ok(_) => false,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return BoardPresence::Unknown(format!("{}: {error}", dot_git.display())),
    };
    let current = located(checkout);
    let git_place = dot_git.join(GIT_STORE_DIR).join(STORE_FILE);
    let places = [
        current.clone(),
        checkout.join(WORK_TREE_STORE_DIR).join(STORE_FILE),
        git_place.clone(),
    ];
    let mut found: Vec<PathBuf> = Vec::new();
    for place in places {
        match std::fs::symlink_metadata(&place) {
            Ok(_) => found.push(place),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error)
                if error.kind() == std::io::ErrorKind::NotADirectory
                    && pointer_resolved
                    && place == git_place => {}
            Err(error) => {
                return BoardPresence::Unknown(format!("{}: {error}", place.display()));
            }
        }
    }
    let displaced = found.iter().find(|place| **place != current).cloned();
    match (found.contains(&current), displaced) {
        (true, displaced) => BoardPresence::Current {
            board: current,
            displaced,
        },
        (false, Some(displaced)) => BoardPresence::Displaced(displaced),
        (false, None) => BoardPresence::Absent,
    }
}

/// The board found only where the layout no longer places it, if any.
#[cfg(test)]
pub(super) fn displaced_board(checkout: &Path) -> Option<PathBuf> {
    match board_presence(checkout) {
        BoardPresence::Displaced(place) => Some(place),
        BoardPresence::Absent | BoardPresence::Current { .. } | BoardPresence::Unknown(_) => None,
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
