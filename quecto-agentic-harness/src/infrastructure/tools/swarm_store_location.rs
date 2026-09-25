//! Where a container's swarm coordination store lives (#2145).
//!
//! In a checkout that is a git repository the store lives in its git
//! directory, `.git/quecto/swarm.sqlite`, where no git command reaches: an
//! agent's `git stash -u`, `git clean -fdx`, `checkout`, `merge` or
//! `reset --hard` deleted or replaced it when it lived in the work tree.
//! Otherwise (a checkout with no repository, or a run started before this
//! layout) it stays at `.quecto/swarm.sqlite`.
//!
//! The location is decided once, by the run's creator, before anything reads
//! the store: it creates `.git/quecto/`, and every process after it finds the
//! store there. Nothing else creates that directory, so a repository an agent
//! initialises mid-run never moves a live store, and a stale board committed
//! at the old path is never adopted.
use std::path::{Path, PathBuf};

/// The store's directory inside a repository's git directory.
const GIT_STORE_DIR: &str = ".git/quecto";
/// The store's directory in the work tree, where no git directory claimed it.
const WORK_TREE_STORE_DIR: &str = ".quecto";
const STORE_FILE: &str = "swarm.sqlite";

/// Swarm artifacts stay in the work tree (their paths are part of the tool's
/// answers), so git is told to leave them alone: never staged by
/// `git add -A`, never removed by `git stash -u` or `git clean -fd`.
pub(super) const ARTIFACT_ENTRIES: [&str; 1] = ["/.quecto/swarm/"];

/// The coordination store of the container whose checkout is `checkout`.
pub(super) fn store_path(checkout: &Path) -> PathBuf {
    let in_git = checkout.join(GIT_STORE_DIR);
    match in_git.is_dir() {
        true => in_git.join(STORE_FILE),
        false => checkout.join(WORK_TREE_STORE_DIR).join(STORE_FILE),
    }
}

/// The creator's one decision: in a checkout whose `.git` is a directory,
/// the store will live in it. Only a run's creator may make it, before the
/// store is read; any other process leaves the location as it finds it.
pub(super) fn claim(checkout: &Path, creator: bool) -> Result<(), String> {
    let git_dir = checkout.join(".git");
    match (creator, git_dir.is_dir()) {
        (true, true) => {
            let dir = checkout.join(GIT_STORE_DIR);
            std::fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))
        }
        (true, false) | (false, _) => Ok(()),
    }
}

/// What excluding the artifacts did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Exclusion {
    Added,
    AlreadyPresent,
    /// The checkout is not the root of a git repository or worktree: the
    /// anchored entries would not apply, so none are written.
    NotARepositoryRoot,
}

/// List the swarm artifacts in the local exclude file of the repository
/// whose root is `checkout`; entries already there are not repeated. The
/// file is replaced whole (written aside, then renamed), so git never reads
/// it part-written.
pub(super) fn exclude_artifacts(checkout: &Path) -> Result<Exclusion, String> {
    let Some(common) = common_git_dir(checkout)? else {
        return Ok(Exclusion::NotARepositoryRoot);
    };
    let exclude = common.join("info/exclude");
    let existing = match std::fs::read_to_string(&exclude) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("{}: {error}", exclude.display())),
    };
    let missing: Vec<&str> = ARTIFACT_ENTRIES
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
    text.push_str("# quecto swarm artifacts (#2145)\n");
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
