//! The startup sweep of the crash directory (#2192 review): a session's
//! records are cleared only when that session runs again, so a session
//! never resumed would keep its records for ever. Each start removes, of
//! every session, the records and leftover temporary files older than
//! [`STALE_RECORD_AGE`] — by the entry's own modification time, only
//! regular files (a link is never followed, nor removed), only names of a
//! record's exact shapes ([`RecordName`]), from a bounded number of
//! entries examined.
use std::ffi::CString;
use std::time::{Duration, SystemTime};

use super::RecordDir;
use super::listing::{self, MAX_LISTED};
use super::names::RecordName;

/// How old a crash record grows before any start removes it: long enough
/// for a parent (or a person) to have read why a process died, short
/// enough that a session never resumed does not keep its records for ever.
pub const STALE_RECORD_AGE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// What one sweep did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Swept {
    /// Stale records removed.
    pub(super) removed: usize,
    /// Whether the directory was listed to its end.
    pub(super) complete: bool,
}

/// Sweep `dir` now, at most [`MAX_LISTED`] entries examined, logging what
/// it did. Never fails: an entry it cannot remove is left, and logged.
pub(super) fn sweep(dir: &RecordDir) {
    let swept = sweep_within(dir, SystemTime::now(), STALE_RECORD_AGE, MAX_LISTED);
    match (swept.removed, swept.complete) {
        (0, true) => {}
        (removed, complete) => tracing::info!(removed, complete, "stale crash records were swept"),
    }
}

/// Remove the records in `dir` older than `age` at `now`, from at most
/// `examined` entries.
pub(super) fn sweep_within(
    dir: &RecordDir,
    now: SystemTime,
    age: Duration,
    examined: usize,
) -> Swept {
    let cutoff = now
        .checked_sub(age)
        .and_then(|cutoff| cutoff.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        });
    let listed =
        listing::names_matching_within(dir, &|name| RecordName::parse(name).is_some(), examined);
    let removed = listed
        .names
        .iter()
        .filter(|name| is_stale(dir, name, cutoff))
        .filter(|name| remove_file(dir, name))
        .count();
    Swept {
        removed,
        complete: listed.complete,
    }
}

/// Whether the entry `name` is a regular file last modified before
/// `cutoff` (seconds since the epoch). A link, a directory, a FIFO — any
/// other kind — is never stale: it is left as it is.
fn is_stale(dir: &RecordDir, name: &str, cutoff: i64) -> bool {
    let Ok(name) = CString::new(name) else {
        return false;
    };
    match dir.entry_stat(&name) {
        Ok(stat) => match stat.st_mode & libc::S_IFMT {
            libc::S_IFREG => {
                let modified: i64 = stat.st_mtime;
                modified < cutoff
            }
            _ => false,
        },
        Err(_) => false,
    }
}

/// Remove the file `name`; answers whether it was removed by this sweep.
fn remove_file(dir: &RecordDir, name: &str) -> bool {
    let Ok(c_name) = CString::new(name) else {
        return false;
    };
    match dir.unlink(&c_name, 0) {
        Ok(()) => true,
        Err(error) => {
            match error.kind() {
                std::io::ErrorKind::NotFound => {}
                _ => tracing::warn!(%error, name, "could not sweep a stale crash record"),
            }
            false
        }
    }
}

#[cfg(test)]
#[path = "crash_record_sweep_tests.rs"]
mod tests;
