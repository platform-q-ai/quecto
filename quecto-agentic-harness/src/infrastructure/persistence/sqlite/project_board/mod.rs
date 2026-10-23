//! SQLite adapter seam for the unchanged project-board port (B2 red phase).
//! No board behavior is implemented here: typed failures keep the contract
//! executable while delivery supplies transactions and canonical codecs.
use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;

use crate::application::project_board::ports::{
    BoardArtifact, BoardBootstrap, BoardHead, BoardSnapshot, BoardStoreError, BoardWrite,
    ProjectBoardStore, SetAsideClear,
};
use crate::domain::project_board::entities::claim::Identity;

pub struct SqliteProjectBoardStore {
    connection: Mutex<Connection>,
    writer: Identity,
}

fn pending() -> BoardStoreError {
    BoardStoreError::Unavailable("SQLite project board not implemented (B2 red)".into())
}

impl SqliteProjectBoardStore {
    /// Opens a real, independent SQLite connection, without implementing B2.
    pub fn open(path: &Path, writer: Identity) -> Result<Self, BoardStoreError> {
        writer
            .validate("writer")
            .map_err(|error| BoardStoreError::InvalidWrite(error.to_string()))?;
        let connection = Connection::open(path)
            .map_err(|error| BoardStoreError::Unavailable(error.to_string()))?;
        Ok(Self {
            connection: Mutex::new(connection),
            writer,
        })
    }

    /// Observe the adapter's own connection, not another connection's defaults.
    #[cfg(feature = "test-support")]
    pub fn connection_settings(&self) -> Result<(String, i64), BoardStoreError> {
        let connection = self.connection.lock().map_err(|_| pending())?;
        let journal = connection.query_row("PRAGMA journal_mode", [], |row| row.get(0));
        let synchronous = connection.query_row("PRAGMA synchronous", [], |row| row.get(0));
        match (journal, synchronous) {
            (Ok(journal), Ok(synchronous)) => Ok((journal, synchronous)),
            _ => Err(pending()),
        }
    }

    /// Fixture seams only; delivery must implement these against durable state,
    /// never a private replay/cache or an alternate in-memory store.
    #[cfg(feature = "test-support")]
    pub fn fixture_put_raw(&self, _path: &str, _bytes: &[u8]) -> Result<(), BoardStoreError> {
        Err(pending())
    }

    #[cfg(feature = "test-support")]
    pub fn fixture_refuse_next_write(&self) -> Result<(), BoardStoreError> {
        Err(pending())
    }

    /// Delivery injects a SQLite failure after the first row of a multi-row
    /// write, inside the real transaction (not a prevalidation refusal).
    #[cfg(feature = "test-support")]
    pub fn fixture_fail_after_first_row(&self) -> Result<(), BoardStoreError> {
        Err(pending())
    }

    #[cfg(feature = "test-support")]
    pub fn fixture_committed_by(&self, _head: &BoardHead) -> Result<Identity, BoardStoreError> {
        Err(pending())
    }
}

impl ProjectBoardStore for SqliteProjectBoardStore {
    fn read(&self) -> Result<BoardSnapshot, BoardStoreError> {
        // Keep the real connection alive; no board has been created by this skeleton.
        let _connection = self.connection.lock().map_err(|_| pending())?;
        Err(BoardStoreError::NoBoard)
    }

    fn write(
        &self,
        _: &BoardHead,
        _: &[BoardArtifact],
        _: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        Err(pending())
    }

    fn clear_set_aside(
        &self,
        _: &BoardHead,
        _: &SetAsideClear,
        _: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        Err(pending())
    }

    fn bootstrap(&self) -> Result<BoardBootstrap, BoardStoreError> {
        Err(pending())
    }

    fn writer(&self) -> Identity {
        self.writer.clone()
    }
}
