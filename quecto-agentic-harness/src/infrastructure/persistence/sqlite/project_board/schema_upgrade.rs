//! The in-place upgrade of a schema-4 board to schema 5 (epic #2705 B11),
//! behind a copy of it beside it: `<file>.schema4-<UTC>` (as
//! `board.sqlite3.schema4-20261026T105649Z`). A copy left by an earlier
//! attempt that checks whole is used again, so a server restarted again
//! and again never fills the disk with copies; a new one is made only with
//! room for it (twice the board and its WAL free), as a file of mode 0600
//! from the start. Any failure refuses the upgrade, leaves the board as it
//! was, removes a copy this attempt made, and says why.
use super::schema::{Schema, schema_of, upgrade};
use super::*;
use crate::infrastructure::persistence::project_board::board_tree::told;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

/// The most copies of one board the same second may name.
const COPY_NAMES: usize = 100;

/// Upgrades the schema-4 board at `path` under `tx`, the write lock held,
/// behind its copy.
///
/// # Errors
/// The upgrade was refused, and why: the board is left as it was.
pub(super) fn upgrade_in_place(
    tx: rusqlite::Transaction<'_>,
    path: &Path,
) -> Result<(), BoardStoreError> {
    let refused = |why: &str| {
        unavailable(format!(
            "the schema-4 board {} was not upgraded to schema 5 and is left as it was: {why}",
            path.display()
        ))
    };
    let (copy, made) =
        kept_copy(path).map_err(|why| refused(&format!("its copy could not be made: {why}")))?;
    let upgraded = upgrade(&tx).and_then(|()| tx.commit().map_err(unavailable));
    match (upgraded, made) {
        (Ok(()), _) => Ok(()),
        (Err(error), true) => {
            remove(&copy);
            Err(refused(&told(&error)))
        }
        (Err(error), false) => Err(refused(&told(&error))),
    }
}

/// A whole copy of the board at `path`: one already beside it, or one made
/// now (and whether it was made now).
fn kept_copy(path: &Path) -> Result<(PathBuf, bool), String> {
    let read = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
    let source = Connection::open_with_flags(path, read).map_err(|e| e.to_string())?;
    source
        .busy_timeout(Duration::from_secs(5))
        .map_err(|e| e.to_string())?;
    if let Some(copy) = copies(path).into_iter().find(|copy| whole(copy, &source)) {
        return Ok((copy, false));
    }
    room(path)?;
    let copy = reserved(path)?;
    match filled(&source, &copy) {
        Ok(()) => Ok((copy, true)),
        Err(why) => {
            remove(&copy);
            Err(why)
        }
    }
}

/// The copies beside the board at `path`, by name: regular files only.
fn copies(path: &Path) -> Vec<PathBuf> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Vec::new();
    };
    let prefix = format!("{}.schema4-", name.to_string_lossy());
    let entries = std::fs::read_dir(dir).into_iter().flatten().flatten();
    let mut found: Vec<PathBuf> = entries
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .map(|entry| entry.path())
        .collect();
    found.sort();
    found
}

/// Whether `copy` is a whole copy of `source`: sound, one file, schema 4,
/// its revisions and objects counted alike.
fn whole(copy: &Path, source: &Connection) -> bool {
    let read = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
    let Ok(kept) = Connection::open_with_flags(copy, read) else {
        return false;
    };
    let text = |sql: &str| kept.query_row(sql, [], |r| r.get::<_, String>(0)).ok();
    let count = "SELECT (SELECT count(*) FROM revisions), (SELECT count(*) FROM objects)";
    let counted = |c: &Connection| -> Option<(i64, i64)> {
        c.query_row(count, [], |r| Ok((r.get(0)?, r.get(1)?))).ok()
    };
    let held = counted(source);
    text("PRAGMA integrity_check(1)").as_deref() == Some("ok")
        && text("PRAGMA journal_mode").as_deref() == Some("delete")
        && matches!(schema_of(&kept), Ok(Schema::Four))
        && held.is_some()
        && counted(&kept) == held
}

/// `Ok` while the board's directory has room for its copy: twice the
/// board and its WAL free.
fn room(path: &Path) -> Result<(), String> {
    let size = |path: &Path| std::fs::metadata(path).map_or(0, |meta| meta.len());
    let mut wal = path.as_os_str().to_owned();
    wal.push("-wal");
    let needed = (size(path) + size(Path::new(&wal))).saturating_mul(2);
    let free = free_bytes(path.parent().unwrap_or(Path::new(".")))?;
    match free >= needed {
        true => Ok(()),
        false => Err(format!(
            "not enough free space beside it: {needed} bytes are needed, {free} are free"
        )),
    }
}

/// The bytes free to this user on the file system holding `dir`.
pub(super) fn free_bytes(dir: &Path) -> Result<u64, String> {
    use std::os::unix::ffi::OsStrExt;
    let name = std::ffi::CString::new(dir.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
    // SAFETY: a NUL-ended path and a buffer of statvfs's own type.
    let done = unsafe { libc::statvfs(name.as_ptr(), stat.as_mut_ptr()) };
    match done {
        0 => {
            // SAFETY: statvfs filled the buffer, as it returned 0.
            let stat = unsafe { stat.assume_init() };
            Ok(wide(stat.f_bavail).saturating_mul(wide(stat.f_frsize)))
        }
        _ => Err(std::io::Error::last_os_error().to_string()),
    }
}

/// `n` as a u64, whatever width the platform gives statvfs's fields.
fn wide(n: impl Into<u64>) -> u64 {
    n.into()
}

/// A new, empty file of mode 0600 named for the board's copy, made now:
/// `<file>.schema4-<UTC, as 20261026T105649Z>` (with `-<n>` when taken).
fn reserved(path: &Path) -> Result<PathBuf, String> {
    let at = humantime::format_rfc3339_seconds(std::time::SystemTime::now()).to_string();
    let compact: String = at.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let stem = format!("{}.schema4-{compact}", path.display());
    for n in 0..COPY_NAMES {
        let candidate = match n {
            0 => PathBuf::from(&stem),
            n => PathBuf::from(format!("{stem}-{n}")),
        };
        let made = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&candidate);
        match made {
            Ok(_) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(format!("{}: {error}", candidate.display())),
        }
    }
    Err(format!("{COPY_NAMES} copies are named {stem}"))
}

/// Fills `copy` with `source` by SQLite's online backup, out of WAL (one
/// file), and checks it whole.
fn filled(source: &Connection, copy: &Path) -> Result<(), String> {
    source
        .backup("main", copy, None)
        .map_err(|e| e.to_string())?;
    let kept = Connection::open(copy).map_err(|e| e.to_string())?;
    let journal: String = kept
        .query_row("PRAGMA journal_mode=DELETE", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    drop(kept);
    match (journal == "delete", whole(copy, source)) {
        (true, true) => Ok(()),
        (_, _) => Err(format!("{} is not a whole copy", copy.display())),
    }
}

/// Removes a copy this attempt made, and any journal SQLite left by it.
fn remove(copy: &Path) {
    let mut journal = copy.as_os_str().to_owned();
    journal.push("-journal");
    let _ = std::fs::remove_file(copy);
    let _ = std::fs::remove_file(Path::new(&journal));
}

#[cfg(test)]
#[path = "schema_upgrade_tests.rs"]
mod tests;
