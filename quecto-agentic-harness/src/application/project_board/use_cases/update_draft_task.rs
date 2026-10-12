//! Edits a draft in place (its title, description, parent and
//! dependencies) as a compare-and-swap, as every change is. Drafts are
//! shared planning: anyone with access to the board edits any draft (it has
//! no holder). A caller that read the draft passes the `updated` it read,
//! and a draft changed since is refused as stale rather than edited over;
//! that refusal is never retried.
//!
//! The edit's own refusals come first (not a draft, nothing changed, the
//! schema's); then only the relations the edit sets are checked, as a new
//! task's are: on the board, none set aside, no cycle closed. A relation
//! the edit leaves alone is never re-checked. Text has CRLF line ends
//! turned to LF. Blocking: a lost write pauses the calling thread before it
//! is tried again.
use std::sync::Arc;

use super::board_change::{checked_writer, commit, task_on};
use super::line_ends::lf;
use super::task_pointers::{Pointers, pointers_hold};
use crate::application::project_board::dto::{DraftUpdate, ProjectBoardError};
use crate::application::project_board::ports::{BoardClock, ProjectBoardStore};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::drafts::{DraftEdit, edit_draft};

pub struct UpdateDraftTask {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
}

impl UpdateDraftTask {
    /// # Errors
    /// The store writes as an invalid identity.
    pub fn new(
        store: Arc<dyn ProjectBoardStore>,
        clock: Arc<dyn BoardClock>,
    ) -> Result<Self, ProjectBoardError> {
        checked_writer(&*store)?;
        Ok(Self { store, clock })
    }

    /// The draft `update` names, with the update applied.
    ///
    /// # Errors
    /// No such task, or its file set aside; a draft changed since it was
    /// read; not a draft, an edit that changes nothing, or the schema's
    /// refusal; a reference it sets not on the board, set aside or closing
    /// a cycle; the clock's; the store's.
    pub fn execute(&self, update: &DraftUpdate) -> Result<Task, ProjectBoardError> {
        let id = &update.id;
        let edit = DraftEdit {
            title: update.title.as_deref().map(lf),
            description: update.description.as_deref().map(lf),
            parent: update.parent.clone(),
            depends_on: update.depends_on.clone(),
        };
        let message = format!("board: edit draft {}", id.as_str());
        let tasks = commit(&*self.store, &*self.clock, &message, |board, when| {
            let task = task_on(board, id)?;
            when.may_build_on(task)?;
            match &update.expected_updated {
                Some(expected) if *expected != task.fields().updated => {
                    return Err(ProjectBoardError::Stale {
                        id: id.clone(),
                        expected: expected.clone(),
                        updated: task.fields().updated.clone(),
                    });
                }
                Some(_) | None => {}
            }
            let edited = edit_draft(task, &edit, &when.now).map_err(ProjectBoardError::Rule)?;
            let set = edited.fields();
            pointers_hold(
                board,
                &Pointers {
                    id,
                    parent: edit.parent.as_ref().and(set.parent.as_ref()),
                    depends_on: match edit.depends_on {
                        Some(_) => &set.depends_on,
                        None => &[],
                    },
                },
            )?;
            Ok(vec![edited])
        })?;
        Ok(tasks.into_iter().next().expect("an edit writes its task"))
    }
}

#[cfg(test)]
#[path = "update_draft_task_tests.rs"]
mod tests;
