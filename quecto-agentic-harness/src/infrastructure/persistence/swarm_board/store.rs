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

use std::ffi::CStr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, Transaction, TransactionBehavior, ffi, types::Value};

use super::py_json::{self, PyJson};
use super::schema::{ADDED_COLUMNS, schema_statements};

/// Python's `sqlite3.connect(..., timeout=0.5)`.
pub const BUSY_TIMEOUT: Duration = Duration::from_millis(500);

/// The most requests the idempotency ledger holds (`Store.retry`).
pub const REQUEST_LEDGER_CAPACITY: i64 = 10_000;

/// The longest request id, in UTF-8 bytes (`bounded(request, 'request id', 128)`).
pub const REQUEST_ID_MAX_BYTES: usize = 128;

const CONTENDED: &str = "coordination store unavailable or contended";

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
        let path = std::path::absolute(&self.path)
            .map_err(|error| StoreRefusal(format!("{CONTENDED}: {error}")))?;
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

/// `sqlite3.connect(path.as_uri() + '?mode=rw[c]', uri=True, timeout=0.5)`.
fn open(path: &Path, create: bool) -> Result<Connection, StoreRefusal> {
    let (mode, flags) = if create {
        (
            "rwc",
            OpenFlags::SQLITE_OPEN_URI
                | OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE,
        )
    } else {
        (
            "rw",
            OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_READ_WRITE,
        )
    };
    let uri = format!("{}?mode={mode}", file_uri(path));
    let connection = Connection::open_with_flags(&uri, flags)
        .map_err(|error| StoreRefusal(format!("{CONTENDED}: {}", opening_message(&error, &uri))))?;
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
    for &byte in path_bytes(path).iter() {
        if byte.is_ascii_alphanumeric() || b"_.-~/".contains(&byte) {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> std::borrow::Cow<'_, [u8]> {
    use std::os::unix::ffi::OsStrExt;
    std::borrow::Cow::Borrowed(path.as_os_str().as_bytes())
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> std::borrow::Cow<'_, [u8]> {
    std::borrow::Cow::Owned(path.to_string_lossy().into_owned().into_bytes())
}

/// `SwarmError(f'coordination store unavailable or contended: {error}')`,
/// where Python's `str(sqlite3.Error)` is `sqlite3_errmsg`.
fn contended(error: &rusqlite::Error) -> StoreRefusal {
    StoreRefusal(format!("{CONTENDED}: {}", sqlite_message(error)))
}

fn sqlite_message(error: &rusqlite::Error) -> String {
    match error {
        rusqlite::Error::SqliteFailure(_, Some(message)) => message.clone(),
        rusqlite::Error::SqliteFailure(failure, None) => error_string(failure.extended_code),
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

/// `Store.retry`: the idempotency ledger. A request seen before replays its
/// stored result when the payload matches; a new one runs `action` and
/// records its result, while the ledger has room.
///
/// # Errors
/// A board refusal for an empty or over-long request id, a reused id with
/// a different payload, or a full ledger; `action`'s error; SQLite errors.
pub fn retry(
    transaction: &Connection,
    actor: &str,
    request: &str,
    payload: &PyJson,
    action: impl FnOnce() -> Result<PyJson, TransactionError>,
) -> Result<PyJson, TransactionError> {
    bounded_request(request)?;
    let payload = encoded(payload)?;
    let stored = transaction
        .prepare("SELECT * FROM requests WHERE actor=? AND request=?")?
        .query_row([actor, request], |row| {
            Ok((
                row.get::<_, Value>("payload")?,
                row.get::<_, Value>("result")?,
            ))
        });
    match stored {
        Ok((stored_payload, stored_result)) => replayed(&stored_payload, &stored_result, &payload),
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            let held: i64 =
                transaction.query_row("SELECT count(*) FROM requests", [], |row| row.get(0))?;
            if held < REQUEST_LEDGER_CAPACITY {
                let result = action()?;
                transaction.execute(
                    "INSERT INTO requests VALUES(?,?,?,?)",
                    [actor, request, payload.as_str(), encoded(&result)?.as_str()],
                )?;
                Ok(result)
            } else {
                Err(TransactionError::board(format!(
                    "coordination request ledger full ({REQUEST_LEDGER_CAPACITY})"
                )))
            }
        }
        Err(error) => Err(error.into()),
    }
}

/// A stored request: its result when the payload text is the same.
fn replayed(
    stored_payload: &Value,
    stored_result: &Value,
    payload: &str,
) -> Result<PyJson, TransactionError> {
    match (stored_payload, stored_result) {
        (Value::Text(stored), Value::Text(result)) if stored == payload => {
            py_json::decode(result).map_err(|error| TransactionError::board(error.to_string()))
        }
        (Value::Text(stored), _) if stored == payload => Err(TransactionError::board(
            "the stored result of this request is not JSON text",
        )),
        _ => Err(TransactionError::board(
            "request id reused with different payload",
        )),
    }
}

/// `Store.event`: one `events` row with the encoded detail.
///
/// # Errors
/// A board refusal for a detail too deep to encode; SQLite errors.
pub fn event(
    transaction: &Connection,
    actor: &str,
    clock_now: f64,
    action: &str,
    detail: &PyJson,
) -> Result<(), TransactionError> {
    transaction.execute(
        "INSERT INTO events(actor,time,action,detail) VALUES(?,?,?,?)",
        rusqlite::params![actor, clock_now, action, encoded(detail)?],
    )?;
    Ok(())
}

/// The board's `encode()`.
fn encoded(value: &PyJson) -> Result<String, TransactionError> {
    py_json::encode(value).map_err(|error| TransactionError::board(error.to_string()))
}

/// `bounded(request, 'request id', 128)`: nonblank by Python's `str.strip`
/// and at most 128 UTF-8 bytes.
fn bounded_request(request: &str) -> Result<(), TransactionError> {
    if has_content(request) && request.len() <= REQUEST_ID_MAX_BYTES {
        Ok(())
    } else {
        Err(TransactionError::board(format!(
            "request id must be nonempty and at most {REQUEST_ID_MAX_BYTES} bytes"
        )))
    }
}

/// Whether `text.strip()` leaves anything, by Python's whitespace.
fn has_content(text: &str) -> bool {
    text.chars().any(|c| !python_whitespace(c))
}

/// Python's `str.isspace`: Unicode White_Space plus the separators
/// U+001C..U+001F, which Python also strips.
fn python_whitespace(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
