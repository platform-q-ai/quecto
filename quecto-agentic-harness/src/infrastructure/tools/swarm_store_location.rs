//! Where a container's swarm coordination store lives (#2145).
//!
//! In a checkout that is a git repository the store lives in its git
//! directory, `.git/quecto/swarm.sqlite`, which git's work-tree commands
//! never touch: an agent's `git stash -u`, `git clean -fdx`, `checkout`,
//! `merge` or `reset --hard` deleted or replaced it when it lived in the work
//! tree. (Re-initialising the repository with a separate git directory moves
//! it; the store then reports itself missing.)
//! Otherwise (a checkout whose `.git` is not a directory, or a run started
//! before this layout) it stays at `.quecto/swarm.sqlite`, listed in the
//! local exclude file where there is one.
//!
//! The location is decided once, by the run's creator, before anything reads
//! the store: it creates `.git/quecto/`, and every process after it finds the
//! store there. Nothing else creates that directory, so a repository an agent
//! initialises mid-run never moves a live store, and a stale board committed
//! at the old path is never adopted.
use std::path::{Path, PathBuf};

/// The store's directory inside a repository's git directory.
const GIT_STORE_DIR: &str = ".git/quecto";
/// The store's directory in the work tree, where no git directory claimed it
/// (no repository, a `.git` that is a file, or a run started before #2145).
const WORK_TREE_STORE_DIR: &str = ".quecto";
const STORE_FILE: &str = "swarm.sqlite";

/// What stays in the work tree is listed in the checkout's local exclude
/// file, so `git add -A` never stages it and `git stash -u` / `git clean -fd`
/// leave it: swarm artifacts (their paths are part of the tool's answers),
/// and a store that no creator could move into the git directory.
pub(super) const WORK_TREE_ENTRIES: [&str; 3] = [
    "/.quecto/swarm/",
    "/.quecto/swarm.sqlite",
    "/.quecto/swarm.sqlite-*",
];

/// The coordination store of the container whose checkout is `checkout`.
pub(super) fn store_path(checkout: &Path) -> PathBuf {
    let in_git = checkout.join(GIT_STORE_DIR);
    match in_git.is_dir() {
        true => in_git.join(STORE_FILE),
        false => checkout.join(WORK_TREE_STORE_DIR).join(STORE_FILE),
    }
}

/// A store file in the work tree, as the creator finds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkTreeStore {
    Absent,
    /// Committed to the repository: a stale board an agent once staged,
    /// never this container's.
    Tracked,
    /// A live board of a run started before this layout.
    Untracked,
}

fn work_tree_store(checkout: &Path) -> WorkTreeStore {
    let relative = Path::new(WORK_TREE_STORE_DIR).join(STORE_FILE);
    if !checkout.join(&relative).exists() {
        return WorkTreeStore::Absent;
    }
    let listed = std::process::Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args(["ls-files", "--error-unmatch", "--"])
        .arg(&relative)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match listed {
        Ok(status) if status.code() == Some(1) => WorkTreeStore::Untracked,
        // Tracked, or git could not say: a stale board is the case that
        // happens (an agent's `git add -A`), so it is never adopted.
        Ok(_) | Err(_) => WorkTreeStore::Tracked,
    }
}

/// The creator's one decision: in a checkout whose `.git` is a directory,
/// the store will live in it. Only a run's creator may make it, before the
/// store is read, and never over a live board already in the work tree (a
/// run started before this layout keeps it); a board committed to the
/// repository is stale and never adopted. Any other process leaves the
/// location as it finds it.
pub(super) fn claim(checkout: &Path, creator: bool) -> Result<(), String> {
    let git_dir = checkout.join(".git");
    match (creator, git_dir.is_dir(), work_tree_store(checkout)) {
        (true, true, WorkTreeStore::Absent | WorkTreeStore::Tracked) => {
            let dir = checkout.join(GIT_STORE_DIR);
            std::fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))
        }
        (true, true, WorkTreeStore::Untracked) | (true, false, _) | (false, _, _) => Ok(()),
    }
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
