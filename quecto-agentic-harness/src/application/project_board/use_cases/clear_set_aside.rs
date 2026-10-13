//! Clears a file the board set aside (#2477): any project member removes
//! it in one change, made by the store's committer, so the board records
//! who did. Its id, if it had one, is free again. A name listed more than
//! once is cleared by naming the object to keep. As every change, it is a
//! compare-and-swap, read again and retried when another writer moved the
//! board first. It reads no clock: nothing it writes is stamped. Blocking:
//! a lost write pauses the calling thread.
use std::sync::Arc;

use super::board_change::{Attempt, checked_writer, retried};
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{
    BoardClock, BoardHead, BoardWrite, KeepCopy, ProjectBoardStore, QuarantinedFile, SetAsideClear,
};
use crate::application::project_board::telemetry::OpFacts;
use std::collections::{BTreeMap, BTreeSet};

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
        self.execute_noted(path, object, repeated, None)
    }

    /// [`Self::execute`], noting in `facts` (epic #2481 T4), when given,
    /// the write's attempts, lost races and pause.
    ///
    /// # Errors
    /// As [`Self::execute`].
    pub fn execute_noted(
        &self,
        path: &str,
        object: &str,
        repeated: &BTreeMap<String, KeepCopy>,
        facts: Option<&mut OpFacts>,
    ) -> Result<BoardHead, ProjectBoardError> {
        let clear = SetAsideClear {
            path: path.to_string(),
            object: object.to_string(),
            repeated: repeated.clone(),
        };
        retried(&*self.clock, facts, || {
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
            if let Some(unseen) = unlisted(&board.quarantined, repeated) {
                return Err(unseen);
            }
            keeps_another(&board.quarantined, path, object, repeated)?;
            // Built only from what the listing shows.
            let message = message(&file.path, repeated);
            match self.store.clear_set_aside(&board.head, &clear, &message)? {
                BoardWrite::Written(head) => {
                    assert_ne!(head, board.head, "a clear the board holds moves its head");
                    Ok(Attempt::Done(head))
                }
                BoardWrite::Lost => Ok(Attempt::Lost),
                BoardWrite::Refused(reason) => Err(ProjectBoardError::Refused(reason)),
            }
        })
    }
}

/// The first name listed more than once, or object of one, that the
/// board's listing does not show, as the error that says so: what a clear
/// names comes from the listing, so anything else is checked away before
/// anything is built from it.
fn unlisted(
    quarantined: &[QuarantinedFile],
    repeated: &BTreeMap<String, KeepCopy>,
) -> Option<ProjectBoardError> {
    repeated.iter().find_map(|(path, keep)| {
        let shown: BTreeSet<&str> = quarantined
            .iter()
            .filter(|file| file.path == *path)
            .map(|file| file.object.as_str())
            .collect();
        let named: Vec<&String> = match keep {
            KeepCopy::Object { keep, seen } => std::iter::once(keep).chain(seen).collect(),
            KeepCopy::Drop(seen) => seen.iter().collect(),
        };
        let unseen = named.iter().find(|object| !shown.contains(object.as_str()));
        match (shown.is_empty(), unseen) {
            (false, None) => None,
            (_, unseen) => Some(ProjectBoardError::NotSetAside {
                path: path.clone(),
                object: unseen
                    .or(named.first())
                    .map_or_else(String::new, |object| (*object).clone()),
            }),
        }
    })
}

/// A clear removes `object` at `path`, so it may keep that same object
/// there only when it is the one object the path holds (copies of one
/// file listed twice): then `object` only names the path's copies. Where
/// the path holds others, keeping the removed object is a contradiction.
fn keeps_another(
    quarantined: &[QuarantinedFile],
    path: &str,
    object: &str,
    repeated: &BTreeMap<String, KeepCopy>,
) -> Result<(), ProjectBoardError> {
    let keeps_its_own =
        matches!(repeated.get(path), Some(KeepCopy::Object { keep, .. }) if keep == object);
    let objects: BTreeSet<&str> = quarantined
        .iter()
        .filter(|file| file.path == path)
        .map(|file| file.object.as_str())
        .collect();
    let only_object = objects.len() == 1 && objects.contains(object);
    match (keeps_its_own, only_object) {
        (false, _) | (true, true) => Ok(()),
        (true, false) => Err(ProjectBoardError::Refused(format!(
            "{path} keeps {object}, the object this clear removes, and holds others: name \
             another of its objects as the clear's object to keep this one"
        ))),
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
