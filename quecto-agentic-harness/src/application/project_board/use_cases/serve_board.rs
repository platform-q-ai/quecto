//! What a board server answers of the one board it serves (epic #2705
//! B4): its status, its time and its stored files at the head.
//!
//! A panic here is fatal: the panic hook aborts the whole server for any
//! panic outside a tool call (ADR-0029). A breach of a port's contract is
//! a [`ServeError`] instead, and nothing indexes or unwraps board data.
use std::sync::Arc;

use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{
    BoardClock, BoardStoreError, ProjectBoardStore, ServeError, ServedSnapshot, ServedStatus,
    ServerRole, SnapshotFiles, SnapshotFilesError,
};
use crate::domain::project_board::value_objects::telemetry::BoardId;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

/// The board server's reads of its board.
pub struct ServeBoard {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    files: Arc<dyn SnapshotFiles>,
    board: BoardId,
}

impl ServeBoard {
    pub fn new(
        store: Arc<dyn ProjectBoardStore>,
        clock: Arc<dyn BoardClock>,
        files: Arc<dyn SnapshotFiles>,
        board: BoardId,
    ) -> Self {
        Self {
            store,
            clock,
            files,
            board,
        }
    }

    pub fn board(&self) -> &BoardId {
        &self.board
    }

    /// The server's role, its board, and the head its store gives.
    ///
    /// # Errors
    /// The store could not be read.
    pub fn status(&self) -> Result<ServedStatus, ServeError> {
        let head = match self.store.read() {
            Ok(snapshot) => Some(snapshot.head),
            Err(BoardStoreError::NoBoard) => None,
            Err(error) => return Err(ServeError::Board(ProjectBoardError::Store(error))),
        };
        Ok(ServedStatus {
            role: ServerRole::Primary,
            board: self.board.clone(),
            head,
        })
    }

    /// What time it is, by the server's clock: the clock authority.
    ///
    /// # Errors
    /// The clock reads a time the board cannot spell.
    pub fn time(&self) -> Result<Timestamp, ServeError> {
        self.clock
            .now()
            .map_err(|error| ServeError::Board(ProjectBoardError::Clock(error)))
    }

    /// `board`'s files at its head, when it is the board served.
    ///
    /// # Errors
    /// Another board, no board, the store's failure, or an artifact no
    /// file holds.
    pub fn snapshot(&self, board: &BoardId) -> Result<ServedSnapshot, ServeError> {
        let snapshot = match *board == self.board {
            true => self.store.read(),
            false => return Err(ServeError::UnknownBoard),
        }
        .map_err(|error| ServeError::Board(ProjectBoardError::Store(error)))?;
        let files = self.files.files(&snapshot).map_err(|error| match error {
            SnapshotFilesError::TooLarge(why) => ServeError::TooLarge(why),
            SnapshotFilesError::Unreadable(why) => ServeError::Files(why),
        })?;
        let read = snapshot.tasks.len()
            + snapshot.plans.len()
            + snapshot.runs.len()
            + snapshot.changes.len()
            + snapshot.reviews.len()
            + snapshot.discussions.len();
        let set_aside = snapshot.quarantined.len();
        let given = files.iter().filter(|file| file.set_aside.is_none()).count();
        // The port's contract: one file per artifact read and one per file
        // set aside. A breach is answered as a fault, never a panic.
        match (given == read, files.len() - given == set_aside) {
            (true, true) => Ok(ServedSnapshot {
                board: self.board.clone(),
                head: snapshot.head,
                files,
            }),
            (false, _) | (_, false) => Err(ServeError::Files(format!(
                "the store gave {} files for {read} artifacts and {set_aside} set aside",
                files.len()
            ))),
        }
    }
}
