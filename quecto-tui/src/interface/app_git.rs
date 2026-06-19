//! Git branch helpers for the TUI footer.

use std::io::Read;
use std::path::{Path, PathBuf};
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
    let git_dir = resolve_git_dir(repo)?;
    let head_path = git_dir.join("HEAD");
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

fn resolve_git_dir(start: &Path) -> Option<PathBuf> {
    for dir in start.ancestors() {
        let dot_git = dir.join(".git");
        if dot_git.is_dir() {
            return Some(dot_git);
        }
        if dot_git.is_file() {
            let gitdir = read_gitdir_file(&dot_git)?;
            return Some(if gitdir.is_absolute() {
                gitdir
            } else {
                dir.join(gitdir)
            });
        }
    }
    None
}

fn read_gitdir_file(path: &Path) -> Option<PathBuf> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.file_type().is_file() || meta.len() > GIT_HEAD_READ_LIMIT {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    let mut contents = String::new();
    file.take(GIT_HEAD_READ_LIMIT)
        .read_to_string(&mut contents)
        .ok()?;
    contents
        .trim()
        .strip_prefix("gitdir:")
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

fn sanitize_git_ref_for_display(ref_name: &str) -> String {
    ref_name
        .chars()
        .filter(|c| !c.is_control() && !is_bidi_or_format_control(*c))
        .collect()
}

fn is_bidi_or_format_control(c: char) -> bool {
    matches!(
        c,
        '\u{00ad}'
            | '\u{061c}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}'
            | '\u{feff}'
    )
}
