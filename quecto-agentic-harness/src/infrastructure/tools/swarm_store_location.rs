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

/// Swarm artifacts stay in the work tree (their paths are part of the tool's
/// answers), listed in the checkout's local exclude file so `git add -A`
/// never stages them and `git stash -u` / `git clean -fd` leave them.
pub(super) const WORK_TREE_ENTRIES: [&str; 1] = ["/.quecto/swarm/"];

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

/// What excluding the work-tree entries did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Exclusion {
    Added,
    AlreadyPresent,
    /// The checkout is not the root of a git repository or worktree: the
    /// anchored entries would not apply, so none are written.
    NotARepositoryRoot,
}

/// List [`WORK_TREE_ENTRIES`] in the local exclude file of the repository
/// whose root is `checkout`; entries already there are not repeated. The
/// file is replaced whole (written aside, then renamed), so git never reads
/// it part-written.
pub(super) fn exclude_work_tree(checkout: &Path) -> Result<Exclusion, String> {
    let Some(common) = common_git_dir(checkout)? else {
        return Ok(Exclusion::NotARepositoryRoot);
    };
    let exclude = common.join("info/exclude");
    let existing = match std::fs::read_to_string(&exclude) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("{}: {error}", exclude.display())),
    };
    let missing: Vec<&str> = WORK_TREE_ENTRIES
        .iter()
        .copied()
        .filter(|entry| !existing.lines().any(|line| line.trim() == *entry))
        .collect();
    if missing.is_empty() {
        return Ok(Exclusion::AlreadyPresent);
    }
    let mut text = existing;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str("# quecto swarm (#2145)\n");
    for entry in missing {
        text.push_str(entry);
        text.push('\n');
    }
    let info = exclude.parent().expect("info/exclude has a parent");
    std::fs::create_dir_all(info).map_err(|error| format!("{}: {error}", info.display()))?;
    let aside = info.join(format!(".exclude.quecto-{}", std::process::id()));
    std::fs::write(&aside, text).map_err(|error| format!("{}: {error}", aside.display()))?;
    std::fs::rename(&aside, &exclude).map_err(|error| {
        let _ = std::fs::remove_file(&aside);
        format!("{}: {error}", exclude.display())
    })?;
    Ok(Exclusion::Added)
}

/// The git directory holding `info/exclude` for the repository or worktree
/// rooted at `checkout`; `None` when `checkout` is no such root.
fn common_git_dir(checkout: &Path) -> Result<Option<PathBuf>, String> {
    let dot_git = checkout.join(".git");
    let metadata = match std::fs::metadata(&dot_git) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("{}: {error}", dot_git.display())),
    };
    if metadata.is_dir() {
        return Ok(Some(dot_git));
    }
    // A linked worktree: `.git` names its git directory, whose `commondir`
    // names the shared one that holds `info/exclude`.
    let pointer = std::fs::read_to_string(&dot_git)
        .map_err(|error| format!("{}: {error}", dot_git.display()))?;
    let Some(named) = pointer.trim().strip_prefix("gitdir:") else {
        return Err(format!("{} names no gitdir", dot_git.display()));
    };
    let git_dir = checkout.join(named.trim());
    let common = match std::fs::read_to_string(git_dir.join("commondir")) {
        Ok(relative) => git_dir.join(relative.trim()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => git_dir,
        Err(error) => return Err(format!("{}: {error}", git_dir.join("commondir").display())),
    };
    Ok(Some(common))
}

#[cfg(test)]
#[path = "swarm_store_location_tests.rs"]
mod tests;
