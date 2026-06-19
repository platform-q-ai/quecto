//! Git branch helpers for the TUI footer.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

/// How often to poll `.git/HEAD` for footer branch changes.
pub(super) const GIT_BRANCH_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Maximum bytes read from `.git/HEAD`; git refs are tiny, so this bounds bad files.
pub(super) const GIT_HEAD_READ_LIMIT: u64 = 4096;

/// Read the current git branch from .git/HEAD.
#[cfg(test)]
pub(super) fn read_git_branch() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    read_git_branch_from(&cwd)
}

pub(super) fn read_git_branch_from(repo: &Path) -> Option<String> {
    let head_path = repo.join(".git/HEAD");
    let meta = std::fs::symlink_metadata(&head_path).ok()?;
    if !meta.file_type().is_file() || meta.len() > GIT_HEAD_READ_LIMIT {
        return None;
    }

    let file = std::fs::File::open(head_path).ok()?;
    let mut head = String::new();
    file.take(GIT_HEAD_READ_LIMIT)
        .read_to_string(&mut head)
        .ok()?;
    let trimmed = head.trim();
    trimmed
        .strip_prefix("ref: refs/heads/")
        .or_else(|| trimmed.strip_prefix("ref: "))
        .map(sanitize_git_ref_for_display)
}

fn sanitize_git_ref_for_display(ref_name: &str) -> String {
    ref_name.chars().filter(|c| !c.is_control()).collect()
}
