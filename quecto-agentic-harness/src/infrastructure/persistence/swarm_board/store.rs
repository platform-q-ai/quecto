//! The swarm board's SQLite store (#2269): the connection discipline of
//! `swarm_helpers/swarm_store.py::Store`, reproduced exactly so that Python
//! and Rust processes can share one board file.
//!
//! Every transaction opens its own connection (no pooling) by URI, with
//! `mode=rw` so a lost board is never recreated, or `mode=rwc` only to
//! create one. It sets the 500 ms busy timeout Python's `timeout=0.5` sets,
//! turns foreign keys on and begins with `BEGIN IMMEDIATE`, so even a read
//! holds the write lock. It sets no other pragma: the journal stays the
//! default rollback journal (WAL would persist in the file). On create it
//! runs the verbatim [`SCHEMA`](super::schema::SCHEMA); on every
//! transaction it adds any missing [`ADDED_COLUMNS`](super::schema::ADDED_COLUMNS).
//!
//! Errors carry Python's text: a SQLite failure is `coordination store
//! unavailable or contended: {sqlite3_errmsg}`, and a board refusal raised
//! inside the transaction rolls it back and is returned unchanged.
//!
//! # Permitted divergences
//!
//! Each is deliberate and pinned by a test (epic #2265 P3):
//! - A relative path under a deleted working directory: Python's
//!   `pathlib` raises `FileNotFoundError`, not a `SwarmError`; here it is
//!   the unavailable refusal with the `getcwd` error (L4).
//! - A non-UTF-8 path in the #2145 missing-store message: Python shows the
//!   byte by `surrogateescape` (`\udcff`), a Rust message shows U+FFFD (N1).
//! - A leading `//`: `pathlib` keeps it (`file:////h/x`); Rust's path
//!   components fold it to one root (`file:///h/x`), the same file (N3).
//! - A SQLite built without the system library's variable limit refuses to
//!   open (Python cannot be built so); see [`variable_limit_checked`].
//! - The stored-result BLOBs of `ledger` (N2) and the REAL-to-TEXT
//!   conversion of `binding` (M1) are recorded in those modules.

use std::ffi::CStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::limits::Limit;
use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior, ffi};

use super::schema::{ADDED_COLUMNS, schema_statements};

/// Python's `sqlite3.connect(..., timeout=0.5)`.
pub const BUSY_TIMEOUT: Duration = Duration::from_millis(500);

pub(super) const CONTENDED: &str = "coordination store unavailable or contended";

/// `SQLITE_MAX_VARIABLE_NUMBER` of the system libsqlite3 Python uses on
/// Debian/Ubuntu and Arch, which `.cargo/config.toml` builds in.
pub const SYSTEM_VARIABLE_LIMIT: i32 = 250_000;

/// A board refusal: the exact message the board raises (Python's
/// `SwarmError`), as the store returns it from a transaction.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct StoreRefusal(pub String);

/// How a transaction's body fails: with a board refusal, returned
/// unchanged, or with a SQLite error, returned as the contended message.
#[derive(Debug, thiserror::Error)]
pub enum TransactionError {
    /// A board refusal (`SwarmError`), returned unchanged.
    #[error("{0}")]
    Board(String),
    /// A SQLite error (`sqlite3.Error`).
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

impl TransactionError {
    /// A board refusal with `message`.
    pub fn board(message: impl Into<String>) -> Self {
        Self::Board(message.into())
    }

    fn into_refusal(self) -> StoreRefusal {
        match self {
            Self::Board(message) => StoreRefusal(message),
            Self::Sqlite(error) => contended(&error),
        }
    }
}

/// The board file one run coordinates through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardStore {
    path: PathBuf,
}

impl BoardStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The path as given (absolutised per transaction, as Python does).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `Store.transaction(create)`: runs `body` in one `BEGIN IMMEDIATE`
    /// transaction on a fresh connection, commits it, and closes the
    /// connection.
    ///
    /// # Errors
    /// The #2145 message for a missing store (checked before opening unless
    /// `create`); `coordination store unavailable or contended: {errmsg}`
    /// for any SQLite error, after rolling back; a [`TransactionError::Board`]
    /// from `body`, unchanged, after rolling back.
    pub fn transaction<T>(
        &self,
        create: bool,
        body: impl FnOnce(&Transaction<'_>) -> Result<T, TransactionError>,
    ) -> Result<T, StoreRefusal> {
        let path = absolutised(&self.path, std::env::current_dir)?;
        // Opened only to create it, or where it is; anything else is a store
        // deleted from under its run (#2145), and mode=rw never recreates it.
        if create || path.exists() {
            let mut connection = open(&path, create)?;
            let outcome = run(&mut connection, create, body);
            debug_assert!(
                connection.is_autocommit(),
                "a board transaction is committed or rolled back before its connection closes"
            );
            let closed = connection.close().map_err(|(_, error)| contended(&error));
            let value = outcome.map_err(TransactionError::into_refusal)?;
            closed.map(|()| value)
        } else {
            Err(StoreRefusal(format!(
                "coordination store missing at {}: it was deleted while the run was live, so this run's board is lost",
                path.display()
            )))
        }
    }
}

/// `pathlib.Path(path).absolute()`: an empty path is the working directory
/// (`current_dir`, consulted only for a relative path), and the path is
/// normalised as pathlib parses it (no `.` components, repeated or trailing
/// separators); `..` is kept, and symlinks are not resolved.
///
/// Permitted divergences: a failing `current_dir` (a deleted working
/// directory) is the unavailable refusal, where Python raises
/// `FileNotFoundError`; a leading `//` folds to one root, where `pathlib`
/// keeps it.
fn absolutised(
    path: &Path,
    current_dir: impl FnOnce() -> std::io::Result<PathBuf>,
) -> Result<PathBuf, StoreRefusal> {
    let given = if path.as_os_str().is_empty() {
        Path::new(".")
    } else {
        path
    };
    let joined = if given.is_absolute() {
        given.to_path_buf()
    } else {
        current_dir()
            .map_err(|error| StoreRefusal(format!("{CONTENDED}: {error}")))?
            .join(given)
    };
    let absolute: PathBuf = joined.components().collect();
    debug_assert!(absolute.is_absolute(), "an absolutised path is absolute");
    Ok(absolute)
}

/// Refuses a SQLite whose variable limit is below the system library's.
///
/// The limit comes from `LIBSQLITE3_FLAGS` in `.cargo/config.toml`, which a
/// build ignores when the variable is already set in its environment. A
/// refusal, not a `debug_assert!`: the build that loses the flag is most
/// likely a release `cargo install`, where an assertion is compiled out and
/// the board would silently refuse `IN (...)` lists Python accepts. Every
/// store failure is a [`StoreRefusal`], so the refusal names the build fault.
fn variable_limit_checked(limit: i32) -> Result<(), StoreRefusal> {
    if limit >= SYSTEM_VARIABLE_LIMIT {
        Ok(())
    } else {
        Err(StoreRefusal(format!(
            "coordination store refused: this build's SQLite binds at most {limit} SQL variables, not the system library's {SYSTEM_VARIABLE_LIMIT} (build it with LIBSQLITE3_FLAGS from .cargo/config.toml)"
        )))
    }
}

/// Refuses a SQLite that does not zero freed pages as the system library does.
///
/// `-DSQLITE_SECURE_DELETE` comes from the same `LIBSQLITE3_FLAGS` as the
/// variable limit, and an exported override can keep one and drop the other,
/// so each is checked on its own. Only the system library's `1` is accepted:
/// `0` leaves deleted board content in freed pages, and `2` (FAST) zeroes
/// only some of them.
fn secure_delete_checked(secure_delete: i64) -> Result<(), StoreRefusal> {
    match secure_delete {
        1 => Ok(()),
        other => Err(StoreRefusal(format!(
            "coordination store refused: this build's SQLite has secure_delete={other}, not the system library's 1 (build it with LIBSQLITE3_FLAGS from .cargo/config.toml)"
        ))),
    }
}

/// The flags `sqlite3.connect(uri, uri=True)` opens an existing board with:
/// a URI filename, read-write. Built with `union` so the combination is one
/// constant that `store_tests.rs` pins bit for bit.
const OPEN_FLAGS: OpenFlags = OpenFlags::SQLITE_OPEN_URI.union(OpenFlags::SQLITE_OPEN_READ_WRITE);

/// [`OPEN_FLAGS`] plus create, for `mode=rwc`.
const CREATE_FLAGS: OpenFlags = OPEN_FLAGS.union(OpenFlags::SQLITE_OPEN_CREATE);

/// `sqlite3.connect(path.as_uri() + '?mode=rw[c]', uri=True, timeout=0.5)`.
fn open(path: &Path, create: bool) -> Result<Connection, StoreRefusal> {
    let (mode, flags) = if create {
        ("rwc", CREATE_FLAGS)
    } else {
        ("rw", OPEN_FLAGS)
    };
    let uri = format!("{}?mode={mode}", file_uri(path));
    let connection = Connection::open_with_flags(&uri, flags)
        .map_err(|error| StoreRefusal(format!("{CONTENDED}: {}", opening_message(&error, &uri))))?;
    let limit = connection
        .limit(Limit::SQLITE_LIMIT_VARIABLE_NUMBER)
        .map_err(|error| contended(&error))?;
    variable_limit_checked(limit)?;
    let secure_delete = connection
        .pragma_query_value(None, "secure_delete", |row| row.get::<_, i64>(0))
        .map_err(|error| contended(&error))?;
    secure_delete_checked(secure_delete)?;
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(|error| contended(&error))?;
    Ok(connection)
}

/// `sqlite3_errmsg` for a failed open. rusqlite appends `: {uri}` to a
/// "cannot open" message, and gives only the URI when SQLite allocated no
/// handle; Python's text has neither.
fn opening_message(error: &rusqlite::Error, uri: &str) -> String {
    match error {
        rusqlite::Error::SqliteFailure(failure, Some(message)) if message == uri => {
            error_string(failure.extended_code)
        }
        rusqlite::Error::SqliteFailure(_, Some(message)) => message
            .strip_suffix(uri)
            .and_then(|message| message.strip_suffix(": "))
            .unwrap_or(message)
            .to_owned(),
        other => sqlite_message(other),
    }
}

/// The transaction proper: pragma, `BEGIN IMMEDIATE`, schema on create,
/// column upgrades, the body, then `COMMIT`; rolled back on any error.
fn run<T>(
    connection: &mut Connection,
    create: bool,
    body: impl FnOnce(&Transaction<'_>) -> Result<T, TransactionError>,
) -> Result<T, TransactionError> {
    connection.execute_batch("PRAGMA foreign_keys=ON")?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let value = match prepared(&transaction, create).and_then(|()| {
        debug_assert!(
            !transaction.is_autocommit(),
            "the board body runs inside BEGIN IMMEDIATE"
        );
        body(&transaction)
    }) {
        Ok(value) => value,
        Err(error) => {
            // The body's error is the one reported; a failed rollback still
            // ends the transaction when the connection closes.
            let _rolled_back = transaction.rollback();
            return Err(error);
        }
    };
    // A failed COMMIT drops the transaction, which rolls it back.
    transaction.commit()?;
    Ok(value)
}

/// The schema on create, then `ensure_columns`.
fn prepared(transaction: &Transaction<'_>, create: bool) -> Result<(), TransactionError> {
    if create {
        for statement in schema_statements() {
            transaction.execute(statement, [])?;
        }
    }
    ensure_columns(transaction)
}

/// `swarm_store.ensure_columns`: for each table in order, the columns it
/// lacks, added in order.
fn ensure_columns(connection: &Connection) -> Result<(), TransactionError> {
    for (table, columns) in ADDED_COLUMNS {
        let present = column_names(connection, table)?;
        for (column, kind) in *columns {
            if present.iter().any(|name| name == column) {
                // Already upgraded: nothing to add.
            } else {
                connection.execute(
                    &format!("ALTER TABLE {table} ADD COLUMN {column} {kind}"),
                    [],
                )?;
            }
        }
    }
    Ok(())
}

fn column_names(connection: &Connection, table: &str) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let names = statement.query_map([], |row| row.get::<_, String>(1))?;
    names.collect()
}

/// `pathlib.Path.as_uri()` for an absolute POSIX path: `file://` and the
/// path's bytes percent-encoded except ASCII letters, digits, `_.-~` and `/`.
fn file_uri(path: &Path) -> String {
    debug_assert!(path.is_absolute(), "only an absolute path has a file URI");
    let mut uri = String::from("file://");
    // The harness is Unix-only (as the Python board is POSIX-only), so a
    // path's bytes are its `OsStr` bytes, non-UTF-8 ones included.
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"_.-~/".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

/// `SwarmError(f'coordination store unavailable or contended: {error}')`,
/// where Python's `str(sqlite3.Error)` is `sqlite3_errmsg`.
fn contended(error: &rusqlite::Error) -> StoreRefusal {
    StoreRefusal(format!("{CONTENDED}: {}", sqlite_message(error)))
}

/// Python's `str()` of the `sqlite3.Error` the same call raises. SQLite's
/// own failures carry `sqlite3_errmsg`; rusqlite adds the SQL and offset to
/// an input error, and words its own checks (the parameter count, one
/// statement per call) differently from Python's `sqlite3` module. The
/// parameter count is Python's only when it carries the true count, as
/// [`bound_statement`](super::binding::bound_statement) reports it.
fn sqlite_message(error: &rusqlite::Error) -> String {
    match error {
        rusqlite::Error::SqliteFailure(_, Some(message)) => message.clone(),
        rusqlite::Error::SqliteFailure(failure, None) => error_string(failure.extended_code),
        rusqlite::Error::SqlInputError { msg, .. } => msg.clone(),
        rusqlite::Error::InvalidParameterCount(given, needed) => format!(
            "Incorrect number of bindings supplied. The current statement uses {needed}, and there are {given} supplied."
        ),
        rusqlite::Error::MultipleStatement => {
            "You can only execute one statement at a time.".to_owned()
        }
        other => other.to_string(),
    }
}

/// `sqlite3_errstr(code)`: SQLite's English text for a result code, the
/// text `sqlite3_errmsg` gives when no more specific message was recorded.
fn error_string(code: std::ffi::c_int) -> String {
    // sqlite3_errstr accepts any integer and returns a pointer to a static,
    // NUL-terminated string (never null) that lives for the whole program.
    // SAFETY: the pointer is non-null, NUL-terminated and 'static.
    let text = unsafe { CStr::from_ptr(ffi::sqlite3_errstr(code)) };
    text.to_string_lossy().into_owned()
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
