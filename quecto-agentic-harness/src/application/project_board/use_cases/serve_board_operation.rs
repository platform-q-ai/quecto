//! A board server's operations (epic #2705 B7): a write is admitted, its
//! host reads made, then it runs once per write id in one transaction under
//! the write lock; a read takes neither. A panic aborts the server
//! (ADR-0029), so every failure is a [`ServeError`].
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

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
    /// The writes admitted, by principal and write id.
    writing: Mutex<BTreeSet<String>>,
}

impl ServeBoardOperation {
    pub fn new(store: Arc<dyn BoardOpStore>, operations: Arc<dyn BoardOperationsOver>) -> Self {
        let writing = Mutex::default();
        Self {
            store,
            operations,
            writing,
        }
    }

    /// The answer `present` gives to `request` (`None`: one its front end
    /// could not read, whose refusal `present` renders, as a read's). A
    /// write needs its `write`, and is admitted before its host reads.
    ///
    /// # Errors
    /// A write without its `write`, its id reused, past [`MAX_WRITES`] or
    /// already running; the store's failure, or operations not built.
    pub fn execute(
        &self,
        request: Option<&BoardRequest>,
        write: Option<&OpWrite>,
        present: &mut Present<'_>,
    ) -> Result<Transacted, ServeError> {
        let failed = |error| ServeError::Board(ProjectBoardError::Store(error));
        let writes = request.map(BoardRequest::writes);
        // Admitted first, so busy is answered before any host is read.
        let _admitted = match (writes, write) {
            (Some(true), Some(write)) => Some(self.admitted(write).ok_or(ServeError::Busy)?),
            (Some(true), None) => return Err(ServeError::NoWrite),
            (Some(false) | None, _) => None,
        };
        let over = self.operations.prepare(request, &*self.store.reads());
        let mut run = |store| {
            let operations = over(store).map_err(|error| error.to_string());
            operations.and_then(|operations| present(&*operations))
        };
        match (writes, write) {
            (Some(true), Some(write)) => {
                // A failed op keeps no answer, so its retry runs it.
                let transacted = self.store.transact(write, &mut |store| {
                    run(store).map_err(BoardStoreError::Unavailable)
                });
                match transacted.map_err(failed)? {
                    Transacted::Reused => Err(ServeError::WriteIdReused),
                    done @ (Transacted::Answered(_) | Transacted::Replayed(_)) => Ok(done),
                }
            }
            (_, _) => run(self.store.reads())
                .map(Transacted::Answered)
                .map_err(|why| failed(BoardStoreError::Unavailable(why))),
        }
    }

    /// `write` admitted: one of [`MAX_WRITES`], its id not already running
    /// (a retry of it is busy, then answered from the kept answer).
    fn admitted(&self, write: &OpWrite) -> Option<Admitted<'_>> {
        let key = format!("{}\n{}", write.principal, write.write_id.as_str());
        let mut held = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
        let free = held.len() < MAX_WRITES && held.insert(key.clone());
        drop(held);
        free.then(|| Admitted(&self.writing, key))
    }
}

/// An admitted write, let go when dropped.
struct Admitted<'a>(&'a Mutex<BTreeSet<String>>, String);

impl Drop for Admitted<'_> {
    fn drop(&mut self) {
        let mut held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        held.remove(&self.1);
    }
}

#[cfg(test)]
#[path = "serve_board_operation_tests.rs"]
mod tests;
