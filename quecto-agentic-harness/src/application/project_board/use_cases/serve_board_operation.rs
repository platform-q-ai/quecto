//! A board server's operations (epic #2705 B7): each `POST /op` is run
//! here, on the server's own store. A write runs once per write id, under
//! the board's write lock, in one immediate transaction that every write
//! it makes lands in (a change and the completion it triggers alike), so
//! its compare-and-swap never loses inside the lock; the same id again is
//! answered with the first answer and writes nothing. A read takes no lock
//! and no transaction. A panic here is fatal: the panic hook aborts the
//! whole server outside a tool call (ADR-0029), so every failure is a
//! [`ServeError`].
use std::sync::Arc;

use crate::application::project_board::dto::{BoardRequest, ProjectBoardError};
use crate::application::project_board::ports::{
    BoardOpStore, BoardOperations, BoardOperationsOver, BoardStoreError, OpWrite, ServeError,
    Transacted,
};

/// What runs a served request on the operations it is handed and gives
/// the answer to send (the server's front end renders it).
pub type Present<'a> = dyn FnMut(&dyn BoardOperations) -> String + 'a;

/// The board server's operations on its board.
pub struct ServeBoardOperation {
    store: Arc<dyn BoardOpStore>,
    operations: Arc<dyn BoardOperationsOver>,
}

impl ServeBoardOperation {
    pub fn new(store: Arc<dyn BoardOpStore>, operations: Arc<dyn BoardOperationsOver>) -> Self {
        Self { store, operations }
    }

    /// The answer `present` gives to `request` (`None`: one its front end
    /// could not read, whose refusal `present` renders, as a read's). A
    /// write needs its `write`; a read takes none.
    ///
    /// # Errors
    /// A write without its `write` ([`ServeError::NoWrite`]); the store's
    /// failure, or operations that could not be built, before any answer.
    pub fn execute(
        &self,
        request: Option<&BoardRequest>,
        write: Option<&OpWrite>,
        present: &mut Present<'_>,
    ) -> Result<Transacted, ServeError> {
        let failed = |error| ServeError::Board(ProjectBoardError::Store(error));
        match (request.map(BoardRequest::writes), write) {
            (Some(true), Some(write)) => {
                // Operations that cannot be built fail the write: it keeps
                // no answer, so its retry runs it.
                let transacted = self.store.transact(write, &mut |store| {
                    let operations = self
                        .operations
                        .over(store)
                        .map_err(|error| BoardStoreError::Unavailable(error.to_string()))?;
                    Ok(present(&*operations))
                });
                transacted.map_err(failed)
            }
            (Some(true), None) => Err(ServeError::NoWrite),
            (Some(false) | None, _) => {
                let operations = self.operations.over(self.store.reads());
                let operations = operations.map_err(ServeError::Board)?;
                Ok(Transacted::Answered(present(&*operations)))
            }
        }
    }
}

#[cfg(test)]
#[path = "serve_board_operation_tests.rs"]
mod tests;
