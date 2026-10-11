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
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{
    BoardClock, BoardHead, BoardWrite, KeepCopy, ProjectBoardStore, QuarantinedFile, SetAsideClear,
};
use std::collections::BTreeMap;

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
    pub fn execute(
        &self,
        path: &str,
        object: &str,
        repeated: &BTreeMap<String, KeepCopy>,
    ) -> Result<BoardHead, ProjectBoardError> {
        let clear = SetAsideClear {
            path: path.to_string(),
            object: object.to_string(),
            repeated: repeated.clone(),
        };

        for attempt in 1..=MAX_WRITE_ATTEMPTS {
            if attempt > 1 {
                self.clock.pause(attempt);
            }
            let board = self.store.read()?;
            // The path is one the board set aside: the store's own list is
            // the check, and the store owns what a name may be.
            let named = |file: &&QuarantinedFile| file.path == path && file.object == object;
            let Some(file) = board.quarantined.iter().find(named) else {
                return Err(ProjectBoardError::NotSetAside {
                    path: path.to_string(),
                    object: object.to_string(),
                });
            };
            let message = message(&file.path, repeated);
            match self.store.clear_set_aside(&board.head, &clear, &message)? {
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

/// The commit message of a clear: the file it clears and, for each name
/// listed more than once, the copy kept (its object's first twelve
/// characters) or that none is. `path` is one the board set aside, so it
/// is already quoted. Past one line's 200 characters, it says only how many
/// it clears.
fn message(path: &str, repeated: &BTreeMap<String, KeepCopy>) -> String {
    let resolved: Vec<String> = repeated
        .iter()
        .map(|(name, keep)| match keep {
            KeepCopy::Object { keep, .. } => {
                let short: String = keep.chars().take(12).collect();
                format!("{name} keeps {short}")
            }
            KeepCopy::Drop(_) => format!("{name} drops every copy"),
        })
        .collect();
    let full = std::iter::once(format!("board: clear set-aside {path}"))
        .chain(resolved)
        .collect::<Vec<_>>()
        .join("; ");
    match (full.chars().count() <= 200, repeated.len()) {
        (true, _) => full,
        (false, 0) => "board: clear set-aside 1 file".into(),
        (false, names) => {
            format!("board: clear set-aside 1 file and {names} names listed more than once")
        }
    }
}

#[cfg(test)]
#[path = "clear_set_aside_tests.rs"]
mod tests;
