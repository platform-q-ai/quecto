//! Durable structural board objects, interpreted by the shared board reader.
use crate::application::project_board::ports::*;
use crate::domain::project_board::{entities::claim::Identity, value_objects::schema_error::line};
use crate::infrastructure::persistence::project_board::board_tree::{
    Board, LIMITS, MAX_BOARD_ENTRIES, object_id,
};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::Mutex,
    time::Duration,
};
#[cfg(feature = "test-support")]
mod fixture;
mod mutation;
mod reader;
mod storage;
#[cfg(feature = "test-support")]
pub use fixture::{FixtureHooks, RawEditor, RawEntry};

pub struct SqliteProjectBoardStore {
    connection: Mutex<Connection>,
    writer: Identity,
    #[cfg(feature = "test-support")]
    payload_fetches: Mutex<Vec<(String, String, usize)>>,
    #[cfg(feature = "test-support")]
    hooks: std::sync::Arc<FixtureHooks>,
}
fn unavailable(e: impl std::fmt::Display) -> BoardStoreError {
    BoardStoreError::Unavailable(e.to_string())
}
fn invalid(e: impl std::fmt::Display) -> BoardStoreError {
    BoardStoreError::InvalidWrite(e.to_string())
}
impl SqliteProjectBoardStore {
    pub fn open(path: &Path, writer: Identity) -> Result<Self, BoardStoreError> {
        writer.validate("writer").map_err(invalid)?;
        Ok(Self {
            connection: Mutex::new(storage::open(path)?),
            writer,
            #[cfg(feature = "test-support")]
            payload_fetches: Mutex::new(vec![]),
            #[cfg(feature = "test-support")]
            hooks: std::sync::Arc::new(FixtureHooks::default()),
        })
    }
}
impl ProjectBoardStore for SqliteProjectBoardStore {
    fn read(&self) -> Result<BoardSnapshot, BoardStoreError> {
        let mut c = self.connection.lock().map_err(unavailable)?;
        let tx = c.transaction().map_err(unavailable)?;
        let (head, root) = storage::current(&tx)?;
        Ok(self.board(&tx, &root)?.snapshot(head))
    }
    fn bootstrap(&self) -> Result<BoardBootstrap, BoardStoreError> {
        use crate::infrastructure::persistence::project_board::{
            board_tree::{FORMAT, FORMAT_FILE},
            store::README,
        };
        let mut c = self.connection.lock().map_err(unavailable)?;
        let tx = c
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(unavailable)?;
        match storage::current(&tx) {
            Ok((head, root)) => {
                self.board(&tx, &root)?;
                Ok(BoardBootstrap::Existing(head))
            }
            Err(BoardStoreError::NoBoard) => {
                let marker = storage::blob(&tx, FORMAT)?;
                let format = storage::tree(
                    &tx,
                    &[storage::entry("100644", FORMAT_FILE.as_bytes(), marker)],
                )?;
                let readme = storage::blob(&tx, README)?;
                let tasks = storage::tree(&tx, &[])?;
                let root = storage::tree(
                    &tx,
                    &[
                        storage::entry("100644", b"README.md", readme),
                        storage::entry("040000", b"format", format),
                        storage::entry("040000", b"tasks", tasks),
                    ],
                )?;
                let head = storage::advance(&tx, &root, &self.writer)?;
                tx.commit().map_err(unavailable)?;
                Ok(BoardBootstrap::Created(head))
            }
            Err(e) => Err(e),
        }
    }
    fn write(
        &self,
        expected: &BoardHead,
        artifacts: &[BoardArtifact],
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        line("message", message, 200).map_err(invalid)?;
        let keys: BTreeSet<_> = artifacts.iter().map(BoardArtifact::key).collect();
        match (artifacts.is_empty(), keys.len() == artifacts.len()) {
            (false, true) => self.change(expected, Some(artifacts), None),
            _ => Err(invalid("write one or more board files, each once")),
        }
    }
    fn clear_set_aside(
        &self,
        expected: &BoardHead,
        clear: &SetAsideClear,
        message: &str,
    ) -> Result<BoardWrite, BoardStoreError> {
        line("message", message, 200).map_err(invalid)?;
        let copies = clear.repeated.values().try_fold(0usize, |count, keep| {
            let seen = match keep {
                KeepCopy::Object { seen, .. } | KeepCopy::Drop(seen) => seen.len(),
            };
            count
                .checked_add(seen)
                .ok_or_else(|| invalid("clear exceeds copy bounds"))
        })?;
        match (
            clear.repeated.len() <= MAX_BOARD_ENTRIES / 2,
            copies <= MAX_BOARD_ENTRIES,
        ) {
            (true, true) => {}
            _ => return Err(invalid("clear exceeds copy bounds")),
        }
        object_id(&clear.object).map_err(|_| invalid("not an object id"))?;
        for keep in clear.repeated.values() {
            let (kept, seen) = match keep {
                KeepCopy::Object { keep, seen } => (Some(keep), seen),
                KeepCopy::Drop(seen) => (None, seen),
            };
            for oid in kept.into_iter().chain(seen) {
                object_id(oid).map_err(|_| invalid("not an object id"))?;
            }
        }
        self.change(expected, None, Some(clear))
    }
    fn writer(&self) -> Identity {
        self.writer.clone()
    }
}
#[cfg(test)]
#[cfg(feature = "test-support")]
mod project_board_sqlite_tests;
