//! Over-long `bash` output saved for the agent to read later (#2167).
//!
//! The directory limits itself: before each save, files the tool itself wrote
//! that are older than a day go, then the oldest go until the save leaves at
//! most [`SAVED_OUTPUT_POLICY`]`.max_files`. Pruning is allowlisted: only
//! regular files named exactly as [`save_output_in`] names them, directly in a
//! real (non-symlink) directory the current user owns, are ever removed. A
//! pruning failure is logged and never fails the save.
//!
//! Off Unix, ownership cannot be proven, so output is never saved there: the
//! model is told so and pointed at `output_file`.
use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Saved files are `bash-output-<6 ASCII letters or digits>.log`.
const NAME_PREFIX: &str = "bash-output-";
const NAME_SUFFIX: &str = ".log";
/// Pinned on the builder so the name pattern never drifts with `tempfile`.
const NAME_RANDOM_LEN: usize = 6;

/// The directory's name; each user has their own (`<name>-<uid>`), so no
/// user can squat another's (#2167 review). Unit tests save under their own
/// name so a test run never prunes the real directory.
#[cfg(not(test))]
const DIR_NAME: &str = "quecto-bash-output";
#[cfg(test)]
const DIR_NAME: &str = "quecto-bash-output-lib-tests";

/// The directory every user shared before it was per user: the files this
/// user saved there are swept away, the directory itself is left alone.
#[cfg(not(test))]
const LEGACY_DIR_NAME: &str = "quecto-bash-output";
#[cfg(test)]
const LEGACY_DIR_NAME: &str = "quecto-bash-output-lib-tests-legacy";

/// The old shared directory keeps a day of this user's files, so a session
/// of an older build still finds what it just saved (#2167 review).
pub(super) const LEGACY_POLICY: PrunePolicy = PrunePolicy {
    max_age: Duration::from_secs(24 * 60 * 60),
    max_files: usize::MAX,
};

/// Where a cut output's tail sits in it, for the note after the tail.
pub(super) struct TailView {
    pub start_line: usize,
    pub end_line: usize,
    pub total: usize,
    pub by_bytes: bool,
    /// The capture itself already dropped part of the middle.
    pub capture_cut: bool,
    pub combined_len: usize,
    pub tail_lines: usize,
    pub tail_bytes: usize,
}

/// The note after a cut output's tail: where the rest was saved, or that it
/// could not be (#2167 review).
pub(super) fn truncation_hint(saved_to: Option<&str>, view: &TailView) -> String {
    match saved_to {
        Some(path) => {
            let limit_note = if view.by_bytes { " (50KB limit)" } else { "" };
            // A capture that dropped its middle is not the full output.
            let saved = match view.capture_cut {
                true => "Output (start and end; middle omitted)",
                false => "Full output",
            };
            format!(
                "\n[Showing lines {}-{} of {}{}. {} ({} bytes) saved to: {}]",
                view.start_line,
                view.end_line,
                view.total,
                limit_note,
                saved,
                view.combined_len,
                path
            )
        }
        None => format!(
            "\n[Output truncated to last {} lines / {} bytes; the full output could not be saved: rerun with \"output_file\" to keep it]",
            view.tail_lines, view.tail_bytes
        ),
    }
}

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
        let temp = std::env::temp_dir();
        sweep_legacy(
            &temp.join(LEGACY_DIR_NAME),
            current_uid(),
            SystemTime::now(),
        );
        let dir = temp.join(format!("{DIR_NAME}-{}", current_uid()));
        create_private_dir(&dir).ok()?;
        let path = save_output_in(&dir, &content, SAVED_OUTPUT_POLICY)?;
        Some(path.display().to_string())
    })
    .await
    .ok()?
}

/// Sweep the old shared directory of this user's expired files, and remove
/// it once it is empty (it then costs nothing more).
/// The old directory was shared: another user may own it while this
/// user's files sit in it (#2187 review), so only this user's own regular
/// files are pruned there, in any real directory; the directory itself is
/// removed only by its owner.
pub(super) fn sweep_legacy(dir: &Path, owner: u32, now: SystemTime) {
    let real_directory = std::fs::symlink_metadata(dir).is_ok_and(|meta| meta.is_dir());
    if real_directory {
        prune_own_files(dir, owner, now, LEGACY_POLICY);
    }
    if is_owned_real_directory(dir, owner) {
        // Fails, harmlessly, while anything is left in it.
        let _ = std::fs::remove_dir(dir);
    }
}

/// Create the saved-output directory readable by its owner alone; an
/// existing one is left as it is (and vetted before each save).
pub(super) fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
}

/// Prune `dir`, then write `content` to a new saved-output file in it. The
/// prune runs first, so the new file is never removed by its own save.
pub(super) fn save_output_in(dir: &Path, content: &str, policy: PrunePolicy) -> Option<PathBuf> {
    save_output_in_as(dir, content, policy, current_uid())
}

/// [`save_output_in`] for `owner`. Output is saved only into a real
/// directory `owner` owns: the temp directory is shared, and a symlink or
/// another user's directory there must never receive a command's output
/// (#2167 review).
pub(super) fn save_output_in_as(
    dir: &Path,
    content: &str,
    policy: PrunePolicy,
    owner: u32,
) -> Option<PathBuf> {
    if !is_owned_real_directory(dir, owner) {
        tracing::warn!(
            dir = %dir.display(),
            "bash output not saved: the directory is not a real directory owned by this user"
        );
        return None;
    }
    // Others must not be able to swap a saved file for their own before the
    // model reads it (#2167 review): the directory is kept owner-only.
    if !make_private(dir) {
        tracing::warn!(dir = %dir.display(), "bash output not saved: the directory cannot be made owner-only");
        return None;
    }
    prune_saved_outputs(dir, owner, SystemTime::now(), policy);
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
    prune_own_files(dir, owner, now, policy)
}

/// Prune `owner`'s saved files directly in `dir`, which the caller has
/// vetted as a real directory.
fn prune_own_files(dir: &Path, owner: u32, now: SystemTime, policy: PrunePolicy) -> usize {
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
    // Room for the file about to be saved: the save leaves at most
    // `max_files` (#2167 review).
    let over_cap = candidates
        .len()
        .saturating_sub(policy.max_files.saturating_sub(1));
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
        // A directory that is not there (the old shared one, usually).
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
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
        // Another save's prune got there first.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "bash output prune: cannot remove file");
            false
        }
    }
}

/// Make an owned directory owner-only (0700), as a directory another build
/// created may not be; true when it is owner-only afterwards.
/// Changed through a handle opened without following a link, so a swapped
/// symlink can never redirect the chmod (#2167 review).
#[cfg(unix)]
pub(super) fn make_private(dir: &Path) -> bool {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    // Already owner-only (checked without following a link): nothing to
    // change, and a directory the owner cannot list still takes saves.
    let private = |meta: std::fs::Metadata| meta.is_dir() && meta.permissions().mode() & 0o077 == 0;
    if std::fs::symlink_metadata(dir).is_ok_and(private) {
        return true;
    }
    let handle = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY)
        .open(dir);
    let Ok(handle) = handle else {
        tracing::warn!(dir = %dir.display(), "bash output: cannot open the directory without following links");
        return false;
    };
    match handle.set_permissions(std::fs::Permissions::from_mode(0o700)) {
        Ok(()) => handle.metadata().is_ok_and(private),
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "bash output: cannot make directory owner-only");
            false
        }
    }
}

#[cfg(not(unix))]
pub(super) fn make_private(_dir: &Path) -> bool {
    false
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
