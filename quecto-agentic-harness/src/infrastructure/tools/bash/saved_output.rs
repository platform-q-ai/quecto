//! Over-long `bash` output saved for the agent to read later (#2167).
//!
//! The directory limits itself: before each save, files the tool itself wrote
//! that are older than a day go, then the oldest go until at most
//! [`SAVED_OUTPUT_POLICY`]`.max_files` remain. Pruning is allowlisted: only
//! regular files named exactly as [`save_output_in`] names them, directly in a
//! real (non-symlink) directory the current user owns, are ever removed. A
//! pruning failure is logged and never fails the save.
use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Saved files are `bash-output-<6 ASCII letters or digits>.log`.
const NAME_PREFIX: &str = "bash-output-";
const NAME_SUFFIX: &str = ".log";
/// Pinned on the builder so the name pattern never drifts with `tempfile`.
const NAME_RANDOM_LEN: usize = 6;

/// The shared directory's name. Unit tests save under their own name so a
/// test run never prunes the real directory.
#[cfg(not(test))]
const DIR_NAME: &str = "quecto-bash-output";
#[cfg(test)]
const DIR_NAME: &str = "quecto-bash-output-lib-tests";

#[derive(Debug, Clone, Copy)]
pub(super) struct PrunePolicy {
    pub max_age: Duration,
    pub max_files: usize,
}

pub(super) const SAVED_OUTPUT_POLICY: PrunePolicy = PrunePolicy {
    max_age: Duration::from_secs(24 * 60 * 60),
    max_files: 200,
};

/// Save content to the shared temp directory asynchronously and return the path.
pub(super) async fn save_to_temp_file(content: String) -> Option<String> {
    tokio::task::spawn_blocking(move || {
        let dir = std::env::temp_dir().join(DIR_NAME);
        std::fs::create_dir_all(&dir).ok()?;
        let path = save_output_in(&dir, &content, SAVED_OUTPUT_POLICY)?;
        Some(path.display().to_string())
    })
    .await
    .ok()?
}

/// Prune `dir`, then write `content` to a new saved-output file in it. The
/// prune runs first, so the new file is never removed by its own save.
pub(super) fn save_output_in(dir: &Path, content: &str, policy: PrunePolicy) -> Option<PathBuf> {
    prune_saved_outputs(dir, current_uid(), SystemTime::now(), policy);
    let mut f = tempfile::Builder::new()
        .prefix(NAME_PREFIX)
        .suffix(NAME_SUFFIX)
        .rand_bytes(NAME_RANDOM_LEN)
        .tempfile_in(dir)
        .ok()?;
    f.write_all(content.as_bytes()).ok()?;
    let (_, path) = f.keep().ok()?;
    debug_assert!(
        path.file_name().is_some_and(is_saved_output_name),
        "a saved file must match the prune allowlist: {}",
        path.display()
    );
    Some(path)
}

/// Whether `name` is exactly `bash-output-<6 ASCII letters or digits>.log`,
/// the only names [`save_output_in`] creates.
pub(super) fn is_saved_output_name(name: &OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let Some(random) = name
        .strip_prefix(NAME_PREFIX)
        .and_then(|rest| rest.strip_suffix(NAME_SUFFIX))
    else {
        return false;
    };
    random.len() == NAME_RANDOM_LEN && random.bytes().all(|b| b.is_ascii_alphanumeric())
}

/// Remove saved-output files in `dir` older than `policy.max_age`, then the
/// oldest until at most `policy.max_files` remain. Only acts when `dir` is a
/// real directory owned by `owner`. Returns how many files were removed.
pub(super) fn prune_saved_outputs(
    dir: &Path,
    owner: u32,
    now: SystemTime,
    policy: PrunePolicy,
) -> usize {
    if !is_owned_real_directory(dir, owner) {
        return 0;
    }
    let mut candidates = match saved_output_files(dir, owner) {
        Ok(candidates) => candidates,
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "bash output prune: cannot list directory");
            return 0;
        }
    };
    // Oldest first; a modification time in the future counts as fresh.
    candidates.sort_by_key(|(modified, _)| *modified);
    let expired = candidates
        .iter()
        .take_while(|(modified, _)| {
            now.duration_since(*modified)
                .is_ok_and(|age| age > policy.max_age)
        })
        .count();
    let over_cap = candidates.len().saturating_sub(policy.max_files);
    let doomed = expired.max(over_cap);
    assert!(doomed <= candidates.len(), "prune count within candidates");
    candidates[..doomed]
        .iter()
        .filter(|(_, path)| remove_saved_output(path))
        .count()
}

/// The directory itself (not through a symlink) exists and `owner` owns it.
pub(super) fn is_owned_real_directory(dir: &Path, owner: u32) -> bool {
    match std::fs::symlink_metadata(dir) {
        Ok(meta) => meta.is_dir() && is_owned_by(&meta, owner),
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "bash output prune: cannot stat directory");
            false
        }
    }
}

/// Every saved-output regular file directly in `dir` owned by `owner`, with
/// its modification time. Unreadable entries are skipped with a warning.
pub(super) fn saved_output_files(
    dir: &Path,
    owner: u32,
) -> std::io::Result<Vec<(SystemTime, PathBuf)>> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                tracing::warn!(dir = %dir.display(), error = %e, "bash output prune: cannot read entry");
                continue;
            }
        };
        if !is_saved_output_name(&entry.file_name()) {
            continue;
        }
        let path = entry.path();
        // `symlink_metadata` never follows a link, so a symlink is not a file.
        let modified = std::fs::symlink_metadata(&path).and_then(|meta| {
            match meta.is_file() && is_owned_by(&meta, owner) {
                true => meta.modified().map(Some),
                false => Ok(None),
            }
        });
        match modified {
            Ok(Some(modified)) => files.push((modified, path)),
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "bash output prune: cannot stat file");
            }
        }
    }
    Ok(files)
}

fn remove_saved_output(path: &Path) -> bool {
    match std::fs::remove_file(path) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "bash output prune: cannot remove file");
            false
        }
    }
}

#[cfg(unix)]
fn is_owned_by(meta: &std::fs::Metadata, owner: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    meta.uid() == owner
}

/// Ownership cannot be proven off Unix, so nothing is ever pruned there.
#[cfg(not(unix))]
fn is_owned_by(_meta: &std::fs::Metadata, _owner: u32) -> bool {
    false
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

#[cfg(not(unix))]
fn current_uid() -> u32 {
    0
}
