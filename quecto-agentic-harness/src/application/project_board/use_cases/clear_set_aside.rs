//! Clears a file the board set aside (#2477): any project member removes
//! it in one change, made by the store's committer, so the board records
//! who did. Its id, if it had one, is free again. A name listed more than
//! once is cleared by naming the object to keep. As every change, it is a
//! compare-and-swap, read again and retried when another writer moved the
//! board first. It reads no clock: nothing it writes is stamped. Blocking:
//! a lost write pauses the calling thread.
use std::sync::Arc;

use super::MAX_WRITE_ATTEMPTS;
use super::board_change::checked_writer;
use crate::application::project_board::dto::{ProjectBoardError, plain_board_file};
use crate::application::project_board::ports::{
    BoardClock, BoardHead, BoardWrite, ProjectBoardStore,
};

pub struct ClearSetAside {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
}

impl ClearSetAside {
    /// # Errors
    /// The store writes as an invalid identity.
    pub fn new(
        store: Arc<dyn ProjectBoardStore>,
        clock: Arc<dyn BoardClock>,
    ) -> Result<Self, ProjectBoardError> {
        checked_writer(&*store)?;
        Ok(Self { store, clock })
    }

    /// The board's head once the file set aside at `path` is gone (or,
    /// for a name listed more than once, only `keep` of it remains).
    ///
    /// # Errors
    /// No file set aside at `path`; a board that kept moving; the store's
    /// refusal or failure.
    pub fn execute(&self, path: &str, keep: Option<&str>) -> Result<BoardHead, ProjectBoardError> {
        let shown = match plain_board_file(path) {
            true => path,
            false => "a file",
        };
        let message = format!("board: clear set-aside {shown}");
        for attempt in 1..=MAX_WRITE_ATTEMPTS {
            if attempt > 1 {
                self.clock.pause(attempt);
            }
            let board = self.store.read()?;
            if !board.quarantined.iter().any(|file| file.path == path) {
                return Err(ProjectBoardError::NotSetAside(path.to_string()));
            }
            match self
                .store
                .clear_set_aside(&board.head, path, keep, &message)?
            {
                BoardWrite::Written(head) => {
                    assert_ne!(head, board.head, "a clear the board holds moves its head");
                    return Ok(head);
                }
                BoardWrite::Lost => continue,
                BoardWrite::Refused(reason) => return Err(ProjectBoardError::Refused(reason)),
            }
        }
        Err(ProjectBoardError::Contended {
            attempts: MAX_WRITE_ATTEMPTS,
        })
    }
}

#[cfg(test)]
#[path = "clear_set_aside_tests.rs"]
mod tests;
