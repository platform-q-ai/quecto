//! A board server's reads of its board (epic #2705 B4): its status and
//! its time. A panic here is fatal: the panic hook aborts the whole server
//! outside a tool call (ADR-0029), so every failure is a [`ServeError`].
use std::sync::Arc;

use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{
    BoardClock, BoardStoreError, ProjectBoardStore, ServeError, ServedStatus, ServerRole,
};
use crate::domain::project_board::value_objects::telemetry::BoardId;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

/// The board server's reads of its board.
pub struct ServeBoard {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    board: BoardId,
}

impl ServeBoard {
    pub fn new(
        store: Arc<dyn ProjectBoardStore>,
        clock: Arc<dyn BoardClock>,
        board: BoardId,
    ) -> Self {
        Self {
            store,
            clock,
            board,
        }
    }

    pub fn board(&self) -> &BoardId {
        &self.board
    }

    /// The server's role, its board, and the head its store gives (none
    /// while there is no board).
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
    /// # Errors
    /// The clock reads a time the board cannot spell.
    pub fn time(&self) -> Result<Timestamp, ServeError> {
        self.clock
            .now()
            .map_err(|error| ServeError::Board(ProjectBoardError::Clock(error)))
    }
}
