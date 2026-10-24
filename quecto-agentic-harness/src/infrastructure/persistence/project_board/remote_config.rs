//! Whether a clone's own config file lists a remote, read as a file with
//! no git process (#2705 B1): what lets a front end that knows its board's
//! location open the store on a remote name without asking git, while git
//! still never takes that name for a path. Only what is plainly shown
//! counts (an allowlist): a clone whose top level is `repo` (`.git` a
//! directory, or a worktree's `gitdir:` pointer and its `commondir`), whose
//! common `config` is a regular file of at most [`MAX_CONFIG_BYTES`], with a
//! `[remote "<name>"]` section spelt exactly so that sets a non-empty
//! `url`. Anything else (a subdirectory, a bare repository, an include, an
//! unusual spelling, a file that cannot be read) is not shown, and the
//! caller asks git instead.
use std::path::{Path, PathBuf};

/// The most of a config file read.
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// Whether the config of the clone whose top level is `repo` plainly lists
/// `remote`, a name already spelt as quecto accepts one.
pub fn lists_remote(repo: &Path, remote: &str) -> bool {
    let spelt = !remote.is_empty()
        && remote
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
    match (spelt, common_dir(repo)) {
        (true, Some(common)) => read_bounded(&common.join("config"))
            .is_some_and(|config| section_sets_url(&config, remote)),
        (false, _) | (true, None) => false,
    }
}

/// The common git directory of the clone whose top level is `repo`: its
/// `.git` directory, or the one a worktree's `.git` file points to (and
/// that directory's `commondir`, when it names one).
fn common_dir(repo: &Path) -> Option<PathBuf> {
    let dot_git = repo.join(".git");
    let meta = std::fs::symlink_metadata(&dot_git).ok()?;
    match (meta.is_dir(), meta.is_file()) {
        (true, _) => Some(dot_git),
        (false, true) => {
            let pointer = read_bounded(&dot_git)?;
            let named = pointer.strip_prefix("gitdir: ")?.trim_end_matches('\n');
            let git_dir = repo.join(named);
            match read_bounded(&git_dir.join("commondir")) {
                Some(common) => Some(git_dir.join(common.trim_end_matches('\n'))),
                None => Some(git_dir),
            }
        }
        (false, false) => None,
    }
}

/// `path`'s text, when it is a regular file of at most
/// [`MAX_CONFIG_BYTES`] of UTF-8.
fn read_bounded(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    match meta.is_file() && meta.len() <= MAX_CONFIG_BYTES {
        true => std::fs::read_to_string(path).ok(),
        false => None,
    }
}

/// Whether `config` holds a `[remote "<remote>"]` header line, spelt
/// exactly so, whose section sets `url` to a non-empty value.
fn section_sets_url(config: &str, remote: &str) -> bool {
    let header = format!("[remote \"{remote}\"]");
    let mut inside = false;
    for line in config.lines().map(str::trim) {
        match line.starts_with('[') {
            true => inside = line == header,
            false if inside => {
                let set = line
                    .split_once('=')
                    .filter(|(key, value)| {
                        key.trim().eq_ignore_ascii_case("url") && !value.trim().is_empty()
                    })
                    .is_some();
                if set {
                    return true;
                }
            }
            false => {}
        }
    }
    false
}

#[cfg(test)]
#[path = "remote_config_tests.rs"]
mod tests;
