//! The swarm board's SQLite store (#2269): red-phase stub.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, Transaction};

use super::py_json::PyJson;

pub const BUSY_TIMEOUT: Duration = Duration::from_millis(500);
pub const REQUEST_LEDGER_CAPACITY: i64 = 10_000;
pub const REQUEST_ID_MAX_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct StoreRefusal(pub String);

#[derive(Debug, thiserror::Error)]
pub enum TransactionError {
    #[error("{0}")]
    Board(String),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
}

impl TransactionError {
    pub fn board(message: impl Into<String>) -> Self {
        Self::Board(message.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardStore {
    path: PathBuf,
}

impl BoardStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn transaction<T>(
        &self,
        _create: bool,
        _body: impl FnOnce(&Transaction<'_>) -> Result<T, TransactionError>,
    ) -> Result<T, StoreRefusal> {
        Err(StoreRefusal("not implemented".into()))
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn file_uri(_path: &Path) -> String {
    String::new()
}

pub fn retry(
    _transaction: &Connection,
    _actor: &str,
    _request: &str,
    _payload: &PyJson,
    _action: impl FnOnce() -> Result<PyJson, TransactionError>,
) -> Result<PyJson, TransactionError> {
    Err(TransactionError::board("not implemented"))
}

pub fn event(
    _transaction: &Connection,
    _actor: &str,
    _clock_now: f64,
    _action: &str,
    _detail: &PyJson,
) -> Result<(), TransactionError> {
    Err(TransactionError::board("not implemented"))
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
