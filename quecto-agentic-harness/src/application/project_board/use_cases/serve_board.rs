//! What a board server answers of the one board it serves (epic #2705
//! B4): its status, its time and its files at the head. Reads only, each
//! through the board's store, clock and file codec as composition gave
//! them, so the same server serves any store.
use std::collections::BTreeSet;
use std::sync::Arc;

use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{
    BoardClock, BoardStoreError, ProjectBoardStore, ServeError, ServedSnapshot, ServedStatus,
    ServerRole, SnapshotFiles,
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

    /// The board this server serves.
    pub fn board(&self) -> &BoardId {
        &self.board
    }

    /// The server's role, its board, and the head its store gives (none
    /// while there is no board).
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
        if *board != self.board {
            return Err(ServeError::UnknownBoard);
        }
        let snapshot = self
            .store
            .read()
            .map_err(|error| ServeError::Board(ProjectBoardError::Store(error)))?;
        let files = self.files.files(&snapshot).map_err(ServeError::Files)?;
        let artifacts = snapshot.tasks.len()
            + snapshot.plans.len()
            + snapshot.runs.len()
            + snapshot.changes.len()
            + snapshot.reviews.len()
            + snapshot.discussions.len();
        let paths: BTreeSet<&str> = files.iter().map(|file| file.path.as_str()).collect();
        // The codec's contract: one file per artifact, each at its own path.
        assert_eq!(files.len(), artifacts, "one file per artifact");
        assert_eq!(paths.len(), files.len(), "each file at its own path");
        Ok(ServedSnapshot {
            board: self.board.clone(),
            head: snapshot.head,
            files,
            set_aside: snapshot.quarantined,
        })
    }
}
