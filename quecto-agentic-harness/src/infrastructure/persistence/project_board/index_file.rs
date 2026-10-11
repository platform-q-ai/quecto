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
//!
//! Readers open the index `immutable`: a published index never changes in
//! place (a rebuild renames a new file, a new inode, over it), so SQLite
//! never looks for a write-ahead log, shared memory or rollback journal
//! beside it, and nothing planted at those names (a forged log, a hot
//! journal, a directory, a FIFO) is applied, waited on or failed for.
//! Every publish and discard removes whatever is at those names, under
//! the writer lock.
//!
//! A reader opens the path itself first, without following a link or
//! waiting on a FIFO, and reads only a regular file. SQLite then opens the
//! path again; something swapped in between those two opens (a FIFO, say)
//! is not seen by the first. The directory is checked to be this user's
//! and writable by no one else ([`private`]), so within it only this
//! user's own processes can swap the file. Whoever may write its parent
//! (`.git/quecto`, which the swarm store shares) could still swap the whole
//! directory between the check and SQLite's open. The window stays open:
//! SQLite resolves links itself, so it cannot be handed the checked file.
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
pub const INDEX_SCHEMA: i32 = 2;
pub const BUSY_TIMEOUT: Duration = Duration::from_secs(2);
/// The longest a writer waits for the writer lock.
pub const LOCK_WAIT: Duration = Duration::from_secs(2);
const LOCK_POLL: Duration = Duration::from_millis(10);
/// The names beside the index where SQLite keeps its own files.
const SIDE_FILES: [&str; 3] = ["-wal", "-shm", "-journal"];
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
CREATE TABLE quarantined (path TEXT NOT NULL, id TEXT, reason TEXT NOT NULL);
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

/// A failure on the reader's path, by an explicit list. A value over the
/// read cap is corrupt. A file SQLite cannot read as this index (not a
/// database, damaged, another format or a plain `SQLITE_ERROR` such as
/// "unsupported file format", unopenable, a read-only rollback or an I/O
/// error from a planted side file) or a value of another type needs a
/// rebuild, which replaces the file and its side files under the writer
/// lock; it carries SQLite's own cause. Any other failure is SQLite's. A
/// build's failures never come here ([`built`]).
impl From<rusqlite::Error> for IndexError {
    fn from(error: rusqlite::Error) -> Self {
        let mistyped = matches!(
            error,
            rusqlite::Error::InvalidColumnType(..)
                | rusqlite::Error::FromSqlConversionFailure(..)
                | rusqlite::Error::IntegralValueOutOfRange(..)
                | rusqlite::Error::Utf8Error(..)
        );
        match (error.sqlite_error_code(), mistyped) {
            (Some(ErrorCode::TooBig), _) => {
                Self::Corrupt(format!("a value over the read cap: {error}"))
            }
            (
                Some(
                    ErrorCode::NotADatabase
                    | ErrorCode::DatabaseCorrupt
                    | ErrorCode::Unknown
                    | ErrorCode::CannotOpen
                    | ErrorCode::ReadOnly
                    | ErrorCode::SystemIoFailure
                    | ErrorCode::SchemaChanged,
                ),
                _,
            )
            | (None, true) => Self::NeedsRebuild(error.to_string()),
            _ => Self::Sqlite(error.to_string()),
        }
    }
}

/// A failure while building an index: never a reason to rebuild it. The
/// file system's failures are I/O; any other is SQLite's.
pub(super) fn built(error: rusqlite::Error) -> IndexError {
    match error.sqlite_error_code() {
        Some(
            ErrorCode::CannotOpen
            | ErrorCode::SystemIoFailure
            | ErrorCode::DiskFull
            | ErrorCode::ReadOnly
            | ErrorCode::PermissionDenied,
        ) => IndexError::Io(format!("building the index: {error}")),
        _ => IndexError::Sqlite(format!("building the index: {error}")),
    }
}

fn io(what: &str, path: &Path, error: &std::io::Error) -> IndexError {
    IndexError::Io(format!("{what} {}: {error}", path.display()))
}

/// The index's directory, private to this user: made (with any missing
/// parent) private if it is missing, then checked as [`private`] checks
/// it.
pub(super) fn directory(index: &Path) -> Result<PathBuf, IndexError> {
    let dir = parent(index)?;
    match DirBuilder::new().recursive(true).mode(0o700).create(dir) {
        Ok(()) => {}
        Err(error) => return Err(io("cannot create", dir, &error)),
    }
    private(dir)?;
    Ok(dir.to_path_buf())
}

fn parent(index: &Path) -> Result<&Path, IndexError> {
    index
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .ok_or_else(|| IndexError::Io(format!("{} has no directory", index.display())))
}

/// Checks `dir` through one handle opened on it, never through a link: a
/// directory of this user's that no one else may write to. One of this
/// user's that others may write to is made private again through that same
/// handle (`fchmod`); a link, anything but a directory, or another user's
/// is refused. Its parent (`.git/quecto`) is shared with the swarm store
/// and may be group-writable, so whoever may write there can remove or
/// rename this directory, but never make a reader take theirs: a directory
/// of theirs, or a link, is refused here.
fn private(dir: &Path) -> Result<(), IndexError> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let refused = |why: &str| IndexError::Io(format!("{} {why}", dir.display()));
    let opened = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(dir);
    let handle = match opened {
        Ok(handle) => handle,
        Err(error) if matches!(error.raw_os_error(), Some(libc::ELOOP | libc::ENOTDIR)) => {
            return Err(refused("is not a directory"));
        }
        Err(error) => return Err(io("cannot open", dir, &error)),
    };
    // SAFETY: geteuid(2) takes nothing and cannot fail.
    let me = unsafe { libc::geteuid() };
    let shape = |handle: &File| -> Result<(bool, bool, bool), IndexError> {
        let meta = handle
            .metadata()
            .map_err(|error| io("cannot inspect", dir, &error))?;
        Ok((meta.is_dir(), meta.uid() == me, meta.mode() & 0o022 == 0))
    };
    match shape(&handle)? {
        (true, true, true) => Ok(()),
        (true, true, false) => {
            handle
                .set_permissions(std::fs::Permissions::from_mode(0o700))
                .map_err(|error| io("cannot make private", dir, &error))?;
            match shape(&handle)? {
                (true, true, true) => Ok(()),
                _ => Err(refused("did not stay private")),
            }
        }
        (true, false, _) => Err(refused("belongs to another user")),
        (false, _, _) => Err(refused("is not a directory")),
    }
}

/// A reader of the index at `path`: a regular file opened read-only,
/// `immutable` and never through a link, with values capped at
/// [`MAX_VALUE_BYTES`].
pub(super) fn reader(path: &Path) -> Result<Connection, IndexError> {
    // The directory first, as writers check it: no index without one.
    let dir = parent(path)?;
    match std::fs::symlink_metadata(dir) {
        Ok(_) => private(dir)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(IndexError::NeedsRebuild("no index directory".into()));
        }
        Err(error) => return Err(io("cannot inspect", dir, &error)),
    }
    // Never through a link, and never waiting on a FIFO.
    let opened = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path);
    let file = match opened {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(IndexError::NeedsRebuild("no index".into()));
        }
        Err(error) if matches!(error.raw_os_error(), Some(libc::ELOOP | libc::ENXIO)) => {
            return Err(IndexError::NeedsRebuild("the index is not a file".into()));
        }
        Err(error) => return Err(io("cannot open", path, &error)),
    };
    match file.metadata() {
        Ok(meta) if meta.file_type().is_file() => {}
        Ok(_) => return Err(IndexError::NeedsRebuild("the index is not a file".into())),
        Err(error) => return Err(io("cannot inspect", path, &error)),
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
        | OpenFlags::SQLITE_OPEN_URI
        | OpenFlags::SQLITE_OPEN_NOFOLLOW
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let uri = immutable_uri(path);
    debug_assert!(uri.starts_with("file:") && uri.ends_with("?immutable=1"));
    let connection = Connection::open_with_flags(&uri, flags)?;
    drop(file);
    connection.busy_timeout(BUSY_TIMEOUT)?;
    let cap = i32::try_from(MAX_VALUE_BYTES).expect("the value cap fits an i32");
    connection.set_limit(Limit::SQLITE_LIMIT_LENGTH, cap)?;
    connection.pragma_update(None, "trusted_schema", false)?;
    Ok(connection)
}

/// `path` as an SQLite URI that opens it immutable: every byte but an
/// unreserved one or `/` percent-encoded.
fn immutable_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file:");
    for byte in path.as_os_str().as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                uri.push(char::from(*byte));
            }
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri.push_str("?immutable=1");
    uri
}

/// The open file is an index of this layout over records of this schema:
/// its version, its exact schema objects, and the record schema it holds.
pub(super) fn is_current(transaction: &Transaction<'_>) -> rusqlite::Result<bool> {
    let version: i32 = transaction.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let ours = version == INDEX_SCHEMA && objects(transaction)? == *EXPECTED;
    if !ours {
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

/// A new index being written beside the old one; removed unless
/// published. Its file stays locked while it lives, so no sweep takes a
/// live writer's build however long it takes.
pub(super) struct Build {
    path: PathBuf,
    published: bool,
    _held: File,
}

impl Build {
    #[cfg(test)]
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
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
    fill: impl FnOnce(&Transaction<'_>) -> rusqlite::Result<()>,
) -> Result<Build, IndexError> {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let count = BUILDS.fetch_add(1, Ordering::Relaxed);
    let path = dir.join(format!(
        "{name}.{}.{nanos}.{count}{BUILD_SUFFIX}",
        std::process::id()
    ));
    let held = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(&path)
        .map_err(|error| io("cannot create", &path, &error))?;
    // A file this process just created alone: nothing else holds it.
    held_now(&held).map_err(|error| io("cannot lock", &path, &error))?;
    let build = Build {
        path,
        published: false,
        _held: held,
    };
    let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
        | OpenFlags::SQLITE_OPEN_NOFOLLOW
        | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let mut connection = Connection::open_with_flags(&build.path, flags).map_err(built)?;
    let filled = (|| {
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
        transaction.commit()
    })();
    filled.map_err(built)?;
    connection.close().map_err(|(_, error)| built(error))?;
    File::open(&build.path)
        .and_then(|file| file.sync_all())
        .map_err(|error| io("cannot flush", &build.path, &error))?;
    Ok(build)
}

/// Moves `build` over the index at `target` (a link there is replaced, not
/// followed) and makes the rename durable.
pub(super) fn publish(mut build: Build, target: &Path, dir: &Path) -> Result<(), IndexError> {
    assert_eq!(
        build.path.parent(),
        Some(dir),
        "a build is published from the index's own directory"
    );
    // A file there is replaced by the rename in one step; anything else
    // (a directory) is removed first.
    match std::fs::symlink_metadata(target) {
        Ok(meta) if meta.file_type().is_dir() => remove_at(target)?,
        Ok(_) | Err(_) => {}
    }
    std::fs::rename(&build.path, target).map_err(|error| io("cannot publish", target, &error))?;
    build.published = true;
    remove_side_files(target)?;
    File::open(dir)
        .and_then(|file| file.sync_all())
        .map_err(|error| io("cannot flush", dir, &error))
}

/// Removes whatever is at the names beside `target` where SQLite keeps its
/// own files, never following a link: a planted log, journal or shared
/// memory, or a directory or FIFO in their place.
fn remove_side_files(target: &Path) -> Result<(), IndexError> {
    for suffix in SIDE_FILES {
        let mut path = target.as_os_str().to_owned();
        path.push(suffix);
        remove_at(&PathBuf::from(path))?;
    }
    Ok(())
}

/// Removes what is at `path` unless it is a regular file: a lock file
/// another writer made there since this one looked stays.
pub(super) fn clear_non_regular(path: &Path) -> Result<(), IndexError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_file() => Ok(()),
        Ok(_) => remove_at(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io("cannot inspect", path, &error)),
    }
}

/// Removes whatever is at `path`, never following a link: a directory
/// with what it holds, anything else itself.
fn remove_at(path: &Path) -> Result<(), IndexError> {
    let removed = match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(error) => Err(error),
    };
    match removed {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io("cannot remove", path, &error)),
    }
}

/// Takes `file`'s lock now, or says why not (another holder included).
//
// `File::try_lock` stabilized in 1.89, the real toolchain floor (see the
// configuration writer's `exclusive_hold`).
#[expect(clippy::incompatible_msrv)]
fn held_now(file: &File) -> std::io::Result<()> {
    file.try_lock().map_err(|error| match error {
        std::fs::TryLockError::WouldBlock => {
            std::io::Error::new(std::io::ErrorKind::WouldBlock, "held by another writer")
        }
        std::fs::TryLockError::Error(error) => error,
    })
}

/// Holds the index's writer lock, a regular file beside it, while alive,
/// waiting at most [`LOCK_WAIT`] for another writer to let it go.
pub(super) fn lock(dir: &Path, name: &str) -> Result<File, IndexError> {
    let path = dir.join(format!("{name}.lock"));
    // Whatever else is there (a link, a directory, a FIFO) is removed, never
    // followed, and the lock file made again, once.
    let file = match lock_file(&path)? {
        Some(file) => file,
        None => {
            clear_non_regular(&path)?;
            lock_file(&path)?.ok_or_else(|| {
                IndexError::Io(format!("{} is not a regular file", path.display()))
            })?
        }
    };
    let deadline = std::time::Instant::now() + LOCK_WAIT;
    loop {
        match held_now(&file) {
            Ok(()) => return Ok(file),
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(LOCK_POLL);
            }
            Err(error) => return Err(io("cannot lock", &path, &error)),
        }
    }
}

/// The lock file at `path`, opened or made without following a link or
/// waiting on a FIFO; `None` when something else is there.
fn lock_file(path: &Path) -> Result<Option<File>, IndexError> {
    let opened = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path);
    let file = match opened {
        Ok(file) => file,
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(libc::ELOOP | libc::EISDIR | libc::ENXIO)
            ) =>
        {
            return Ok(None);
        }
        Err(error) => return Err(io("cannot open", path, &error)),
    };
    match file.metadata() {
        Ok(meta) if meta.file_type().is_file() => Ok(Some(file)),
        Ok(_) => Ok(None),
        Err(error) => Err(io("cannot inspect", path, &error)),
    }
}

/// Removes build files of `name` that a process left when it died mid-build:
/// regular files older than [`ABANDONED_AFTER`] that no live writer holds
/// locked. A build in progress is locked by its writer however old it is,
/// so it is never touched.
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
        if ours && abandoned && unheld(&entry.path()) {
            // Best effort: a file another sweep removed first is gone anyway.
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// Whether no live writer holds the build file at `path` (a regular file,
/// opened without following a link or waiting on a FIFO).
fn unheld(path: &Path) -> bool {
    let opened = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path);
    opened.is_ok_and(|file| {
        file.metadata().is_ok_and(|meta| meta.file_type().is_file()) && held_now(&file).is_ok()
    })
}

/// Removes the index at `target` (a link there is removed, not followed)
/// and its side files.
pub(super) fn discard(target: &Path) -> Result<(), IndexError> {
    remove_at(target)?;
    remove_side_files(target)
}
