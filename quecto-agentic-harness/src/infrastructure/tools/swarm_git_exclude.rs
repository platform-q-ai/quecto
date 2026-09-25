//! Keep the swarm's coordination store out of git's reach (#2145).
//! The store lives in the checkout (`.quecto/swarm.sqlite`, see
//! `swarm_bridge::store_database`). Where a repository tracks `.quecto/` (a
//! committed standard container), git treats it as an untracked file, so an
//! agent's `git stash -u` or `git clean -fd` deletes it and `git add -A`
//! stages it. Listing it in the checkout's local exclude file stops all
//! three; nothing is committed.
use std::path::{Path, PathBuf};

/// Anchored at the repository root: the store, its journal files and swarm
/// artifacts.
pub(super) const STORE_ENTRIES: [&str; 3] = [
    "/.quecto/swarm.sqlite",
    "/.quecto/swarm.sqlite-*",
    "/.quecto/swarm/",
];

/// What excluding did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Excluded {
    Added,
    AlreadyExcluded,
    /// The checkout is not the root of a git repository or worktree: the
    /// anchored entries would not apply, so none are written.
    NotARepositoryRoot,
}

/// List the store in the local exclude file of the repository whose root
/// is `checkout`; entries already present are not repeated.
pub(super) fn exclude_from_git(checkout: &Path) -> Result<Excluded, String> {
    let Some(common) = common_git_dir(checkout)? else {
        return Ok(Excluded::NotARepositoryRoot);
    };
    let exclude = common.join("info/exclude");
    let existing = match std::fs::read_to_string(&exclude) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("{}: {error}", exclude.display())),
    };
    let missing: Vec<&str> = STORE_ENTRIES
        .iter()
        .copied()
        .filter(|entry| !existing.lines().any(|line| line.trim() == *entry))
        .collect();
    if missing.is_empty() {
        return Ok(Excluded::AlreadyExcluded);
    }
    let mut text = existing;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str("# quecto swarm coordination store (#2145)\n");
    for entry in missing {
        text.push_str(entry);
        text.push('\n');
    }
    let info = exclude.parent().expect("info/exclude has a parent");
    std::fs::create_dir_all(info).map_err(|error| format!("{}: {error}", info.display()))?;
    std::fs::write(&exclude, text).map_err(|error| format!("{}: {error}", exclude.display()))?;
    Ok(Excluded::Added)
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
#[path = "swarm_git_exclude_tests.rs"]
mod tests;
