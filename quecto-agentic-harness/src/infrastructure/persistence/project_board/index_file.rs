//! The board index on disk. The index lives in a private directory that is
//! never a link. A rebuild is written whole to a fresh file beside the
//! index and renamed over it under a lock. Readers open only a regular
//! file, never through a link, whose schema is exactly the index's own.
//!
//! The index is never changed in place, so nothing planted in an old file
//! survives a rebuild: no trigger, view or odd table. A file that is not a
//! database, or holds another schema, is never written to. A rebuild never
//! frees pages inside a file that keeps living, so the index needs no
//! VACUUM.
use super::index::IndexError;
use super::task_record::{MAX_TASK_FILE_BYTES, SCHEMA_VERSION};
use rusqlite::limits::Limit;
use rusqlite::{Connection, ErrorCode, OpenFlags, Transaction};
use std::fs::{DirBuilder, File, OpenOptions};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// The index's own layout version, in `PRAGMA user_version`.
pub const INDEX_SCHEMA: i32 = 1;
pub const BUSY_TIMEOUT: Duration = Duration::from_secs(2);
/// The longest value a reader takes: a task record at its cap, with room
/// to spare.
pub const MAX_VALUE_BYTES: usize = MAX_TASK_FILE_BYTES + 64 * 1024;
/// A build file older than this was left by a process that died mid-build.
const ABANDONED_AFTER: Duration = Duration::from_secs(10 * 60);
const BUILD_SUFFIX: &str = ".build";

pub(super) const SCHEMA: &str = "
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE tasks (
    id TEXT PRIMARY KEY,
    status TEXT NOT NULL,
    parent TEXT,
    claimed INTEGER NOT NULL CHECK (claimed IN (0, 1)),
    record BLOB NOT NULL
);
CREATE INDEX tasks_status ON tasks (status);
CREATE INDEX tasks_parent ON tasks (parent);
CREATE TABLE dependencies (task TEXT NOT NULL, depends_on TEXT NOT NULL, PRIMARY KEY (task, depends_on));
CREATE INDEX dependencies_on ON dependencies (depends_on);
CREATE TABLE quarantined (path TEXT PRIMARY KEY, id TEXT, reason TEXT NOT NULL);
";

type Object = (String, String, String, Option<String>);

/// Every schema object of a fresh index, as `sqlite_master` lists them.
static EXPECTED: LazyLock<Vec<Object>> = LazyLock::new(|| {
    let connection = Connection::open_in_memory().expect("an in-memory database");
    connection.execute_batch(SCHEMA).expect("the index schema");
    objects(&connection).expect("the index schema reads back")
});

fn objects(connection: &Connection) -> rusqlite::Result<Vec<Object>> {
    let mut statement = connection
        .prepare("SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name")?;
    let rows = statement.query_map([], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
    })?;
    rows.collect()
}

/// The file is not an index of this layout, or not a database at all.
pub(super) fn stale(error: &rusqlite::Error) -> bool {
    matches!(
        error.sqlite_error_code(),
        Some(ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt | ErrorCode::CannotOpen)
    )
}

impl From<rusqlite::Error> for IndexError {
    fn from(error: rusqlite::Error) -> Self {
        match error.sqlite_error_code() {
            Some(ErrorCode::TooBig) => Self::Corrupt(format!("a value over the read cap: {error}")),
            _ if stale(&error) => Self::NeedsRebuild,
            _ => Self::Sqlite(error.to_string()),
        }
    }
}

fn io(what: &str, path: &Path, error: &std::io::Error) -> IndexError {
    IndexError::Io(format!("{what} {}: {error}", path.display()))
}

/// The index's directory, made private if it is missing; refused if it is
/// anything but a real directory.
pub(super) fn directory(index: &Path) -> Result<PathBuf, IndexError> {
    let dir = index
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .ok_or_else(|| IndexError::Io(format!("{} has no directory", index.display())))?;
    match std::fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_dir() => return Ok(dir.to_path_buf()),
        Ok(_) => {
            return Err(IndexError::Io(format!(
                "{} is not a directory",
                dir.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(io("cannot inspect", dir, &error)),
    }
    match DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(io("cannot create", dir, &error)),
    }
    match std::fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_dir() => Ok(dir.to_path_buf()),
        _ => Err(IndexError::Io(format!(
            "{} is not a directory",
            dir.display()
        ))),
    }
}

/// A reader of the index at `path`: a regular file opened read-only and
/// never through a link, with values capped at [`MAX_VALUE_BYTES`].
pub(super) fn reader(path: &Path) -> Result<Connection, IndexError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_file() => {}
        Ok(_) => return Err(IndexError::NeedsRebuild),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(IndexError::NeedsRebuild);
        }
        Err(error) => return Err(io("cannot inspect", path, &error)),
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_NOFOLLOW
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = Connection::open_with_flags(path, flags)?;
    connection.busy_timeout(BUSY_TIMEOUT)?;
    let cap = i32::try_from(MAX_VALUE_BYTES).expect("the value cap fits an i32");
    connection.set_limit(Limit::SQLITE_LIMIT_LENGTH, cap)?;
    connection.pragma_update(None, "trusted_schema", false)?;
    Ok(connection)
}

/// The open file is an index of this layout over records of this schema:
/// its version, its exact schema objects, and the record schema it holds.
pub(super) fn is_current(transaction: &Transaction<'_>) -> rusqlite::Result<bool> {
    let version: i32 = transaction.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version != INDEX_SCHEMA || objects(transaction)? != *EXPECTED {
        return Ok(false);
    }
    let mut statement =
        transaction.prepare("SELECT value FROM meta WHERE key = 'record_schema'")?;
    let record: Option<String> = statement
        .query_map([], |row| row.get(0))?
        .next()
        .transpose()?;
    Ok(record == Some(SCHEMA_VERSION.to_string()))
}

/// A new index being written beside the old one; removed unless published.
pub(super) struct Build {
    path: PathBuf,
    published: bool,
}

impl Drop for Build {
    fn drop(&mut self) {
        if !self.published {
            // A build that never published is this process's own file.
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

static BUILDS: AtomicU64 = AtomicU64::new(0);

/// Writes a whole new index to a fresh file in `dir` (created exclusively,
/// never through a link): the layout, then `fill`, then flushed to disk.
pub(super) fn build(
    dir: &Path,
    name: &str,
    fill: impl FnOnce(&Transaction<'_>) -> Result<(), IndexError>,
) -> Result<Build, IndexError> {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let count = BUILDS.fetch_add(1, Ordering::Relaxed);
    let path = dir.join(format!(
        "{name}.{}.{nanos}.{count}{BUILD_SUFFIX}",
        std::process::id()
    ));
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
        .map_err(|error| io("cannot create", &path, &error))?;
    let build = Build {
        path,
        published: false,
    };
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_NOFOLLOW
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut connection = Connection::open_with_flags(&build.path, flags)?;
    connection.pragma_update_and_check(None, "journal_mode", "MEMORY", |row| {
        row.get::<_, String>(0)
    })?;
    let transaction = connection.transaction()?;
    transaction.execute_batch(SCHEMA)?;
    transaction.pragma_update(None, "user_version", INDEX_SCHEMA)?;
    transaction.execute(
        "INSERT INTO meta (key, value) VALUES ('record_schema', ?1)",
        [SCHEMA_VERSION.to_string()],
    )?;
    fill(&transaction)?;
    transaction.commit()?;
    connection
        .close()
        .map_err(|(_, error)| IndexError::from(error))?;
    File::open(&build.path)
        .and_then(|file| file.sync_all())
        .map_err(|error| io("cannot flush", &build.path, &error))?;
    Ok(build)
}

/// Moves `build` over the index at `target` (a link there is replaced, not
/// followed) and makes the rename durable.
pub(super) fn publish(mut build: Build, target: &Path, dir: &Path) -> Result<(), IndexError> {
    std::fs::rename(&build.path, target).map_err(|error| io("cannot publish", target, &error))?;
    build.published = true;
    File::open(dir)
        .and_then(|file| file.sync_all())
        .map_err(|error| io("cannot flush", dir, &error))
}

/// Holds the index's writer lock, a regular file beside it, while alive.
//
// `File::lock` stabilized in 1.89, the real toolchain floor (see the
// configuration writer's `exclusive_hold`).
#[expect(clippy::incompatible_msrv)]
pub(super) fn lock(dir: &Path, name: &str) -> Result<File, IndexError> {
    let path = dir.join(format!("{name}.lock"));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(&path)
        .map_err(|error| io("cannot open", &path, &error))?;
    match file.metadata() {
        Ok(meta) if meta.file_type().is_file() => {}
        _ => {
            return Err(IndexError::Io(format!(
                "{} is not a regular file",
                path.display()
            )));
        }
    }
    file.lock()
        .map_err(|error| io("cannot lock", &path, &error))?;
    Ok(file)
}

/// Removes build files of `name` that a process left when it died mid-build:
/// regular files older than [`ABANDONED_AFTER`]. A build in progress is
/// younger, so it is never touched.
pub(super) fn sweep(dir: &Path, name: &str) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let prefix = format!("{name}.");
    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let ours = file_name
            .to_str()
            .is_some_and(|text| text.starts_with(&prefix) && text.ends_with(BUILD_SUFFIX));
        let abandoned = std::fs::symlink_metadata(entry.path()).is_ok_and(|meta| {
            meta.file_type().is_file()
                && meta
                    .modified()
                    .ok()
                    .and_then(|at| at.elapsed().ok())
                    .is_some_and(|age| age > ABANDONED_AFTER)
        });
        if ours && abandoned {
            // Best effort: a file another sweep removed first is gone anyway.
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Removes the index at `target` (a link there is removed, not followed).
pub(super) fn discard(target: &Path) -> Result<(), IndexError> {
    match std::fs::remove_file(target) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io("cannot remove", target, &error)),
    }
}
