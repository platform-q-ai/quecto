//! Edits a draft in place (its title, description, plan, parent and
//! dependencies) as a compare-and-swap, as every change is: the edit runs
//! against the draft as last read, so a draft that moved on meanwhile is
//! refused. Its new references hold as a new task's do: on the board, none
//! set aside, no cycle closed. Anyone with access edits a draft: it has no
//! holder. Blocking: a lost write pauses the calling thread before it is
//! tried again.
use std::sync::Arc;

use super::board_change::{checked_writer, commit, task_on};
use super::references::{References, references_hold};
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{BoardClock, ProjectBoardStore};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::drafts::{DraftEdit, edit_draft};
use crate::domain::project_board::value_objects::slug::Slug;

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

    /// Draft `id` with `edit` applied.
    ///
    /// # Errors
    /// No such task, or its file set aside; a reference not on the board,
    /// set aside or closing a cycle; a task that is not a draft, an empty
    /// edit or the schema's refusal; the clock's; the store's.
    pub fn execute(&self, id: &Slug, edit: &DraftEdit) -> Result<Task, ProjectBoardError> {
        let message = format!("board: edit draft {}", id.as_str());
        let tasks = commit(&*self.store, &*self.clock, &message, |board, when| {
            let task = task_on(board, id)?;
            when.may_build_on(task)?;
            let parent = edit
                .parent
                .clone()
                .unwrap_or_else(|| task.fields().parent.clone());
            let depends_on = edit
                .depends_on
                .clone()
                .unwrap_or_else(|| task.fields().depends_on.clone());
            references_hold(
                board,
                &References {
                    id,
                    parent: parent.as_ref(),
                    depends_on: &depends_on,
                },
            )?;
            Ok(vec![
                edit_draft(task, edit, &when.now).map_err(ProjectBoardError::Rule)?,
            ])
        })?;
        Ok(tasks.into_iter().next().expect("an edit writes its task"))
    }
}

#[cfg(test)]
#[path = "update_draft_task_tests.rs"]
mod tests;
