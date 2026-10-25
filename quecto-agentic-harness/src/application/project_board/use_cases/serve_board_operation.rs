//! A board server's operations (epic #2705 B7). A panic aborts the server
//! (ADR-0029), so every failure is a [`ServeError`].
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, PoisonError};

use crate::application::project_board::dto::{BoardRequest, ProjectBoardError};
use crate::application::project_board::ports::{
    BoardOpStore, BoardOperations, BoardOperationsOver, BoardStoreError, OpWrite, ServeError,
    ServedRun, Transacted,
};

/// The answer the front end renders on the operations handed, or why the
/// store failed (a write is then undone and keeps no answer).
pub type Present<'a> = dyn FnMut(&dyn BoardOperations) -> Result<String, String> + 'a;

/// The writes held at once (one runs, one waits on the lock); more are busy.
pub const MAX_WRITES: usize = 2;

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

    /// `present`'s answer to `request` (`None`: unread, refused as a read).
    /// A write is admitted, replayed if kept, its host reads made, then run
    /// once per write id in one transaction under the write lock.
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
        self.execute_run(request, write, &ServedRun::Plain, present)
    }

    /// [`Self::execute`], with what the client sent for the request's
    /// `run` (B8), `sent`: an end's facts or a start's undo.
    ///
    /// # Errors
    /// As [`Self::execute`].
    pub fn execute_run(
        &self,
        request: Option<&BoardRequest>,
        write: Option<&OpWrite>,
        sent: &ServedRun,
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
        // A write already answered is replayed with no host read.
        if let (Some(true), Some(write)) = (writes, write) {
            match self.store.kept(write).map_err(failed)? {
                Some(Transacted::Reused) => return Err(ServeError::WriteIdReused),
                Some(replayed) => return Ok(replayed),
                None => {}
            }
        }
        let over = self.operations.prepare(request, sent, &*self.store.reads());
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
