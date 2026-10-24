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
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::application::project_board::dto::{BoardRequest, ProjectBoardError};
use crate::application::project_board::ports::{
    BoardOpStore, BoardOperations, BoardOperationsOver, BoardStoreError, OpWrite, ServeError,
    Transacted,
};

/// Runs a served request on the operations it is handed and gives the
/// answer to send (the server's front end renders it), or why the store
/// failed, which undoes a write and keeps no answer.
pub type Present<'a> = dyn FnMut(&dyn BoardOperations) -> Result<String, String> + 'a;

/// The writes a server holds at once: one runs, one waits on the lock;
/// more are answered busy, so writes never take the threads reads need.
pub const MAX_WRITES: usize = 2;

/// The board server's operations on its board.
pub struct ServeBoardOperation {
    store: Arc<dyn BoardOpStore>,
    operations: Arc<dyn BoardOperationsOver>,
    writing: AtomicUsize,
}

impl ServeBoardOperation {
    pub fn new(store: Arc<dyn BoardOpStore>, operations: Arc<dyn BoardOperationsOver>) -> Self {
        let writing = AtomicUsize::new(0);
        Self {
            store,
            operations,
            writing,
        }
    }

    /// The answer `present` gives to `request` (`None`: one its front end
    /// could not read, whose refusal `present` renders, as a read's). A
    /// write needs its `write`; a read takes none.
    ///
    /// # Errors
    /// A write without its `write`, its id reused, or past [`MAX_WRITES`];
    /// the store's failure, or operations that could not be built.
    pub fn execute(
        &self,
        request: Option<&BoardRequest>,
        write: Option<&OpWrite>,
        present: &mut Present<'_>,
    ) -> Result<Transacted, ServeError> {
        let failed = |error| ServeError::Board(ProjectBoardError::Store(error));
        let over = self.operations.prepare(request, &*self.store.reads());
        let mut run = |store| {
            let operations = over(store).map_err(|error| error.to_string());
            operations.and_then(|operations| present(&*operations))
        };
        match (request.map(BoardRequest::writes), write) {
            (Some(true), Some(write)) => {
                let _slot = Slot::taken(&self.writing).ok_or(ServeError::Busy)?;
                // A failed op keeps no answer, so its retry runs it.
                let transacted = self.store.transact(write, &mut |store| {
                    run(store).map_err(BoardStoreError::Unavailable)
                });
                match transacted.map_err(failed)? {
                    Transacted::Reused => Err(ServeError::WriteIdReused),
                    done @ (Transacted::Answered(_) | Transacted::Replayed(_)) => Ok(done),
                }
            }
            (Some(true), None) => Err(ServeError::NoWrite),
            (Some(false) | None, _) => run(self.store.reads())
                .map(Transacted::Answered)
                .map_err(|why| failed(BoardStoreError::Unavailable(why))),
        }
    }
}

/// One of the [`MAX_WRITES`] write slots, given back when dropped.
struct Slot<'a>(&'a AtomicUsize);

impl<'a> Slot<'a> {
    fn taken(writing: &'a AtomicUsize) -> Option<Self> {
        let free = |held: usize| (held < MAX_WRITES).then_some(held + 1);
        let taken = writing.fetch_update(Ordering::SeqCst, Ordering::SeqCst, free);
        taken.ok().map(|_| Self(writing))
    }
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
#[path = "serve_board_operation_tests.rs"]
mod tests;
