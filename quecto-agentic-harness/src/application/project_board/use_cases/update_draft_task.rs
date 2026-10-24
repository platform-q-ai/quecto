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

use super::board_change::{checked_writer, commit, task_on, written_task};
use super::task_pointers::{Pointers, pointers_hold};
use crate::application::project_board::dto::{DraftUpdate, ProjectBoardError};
use crate::application::project_board::ports::{BoardArtifact, BoardClock, ProjectBoardStore};
use crate::application::project_board::telemetry::OpFacts;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::drafts::{DraftEdit, edit_draft};
use crate::domain::project_board::value_objects::board_text;
use crate::domain::project_board::value_objects::telemetry::Normalised;

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
        self.execute_noted(update, None)
    }

    /// [`Self::execute`], noting in `facts` (epic #2481 T4), when given,
    /// the write's attempts, the draft's status as last read and as
    /// written, and what the normaliser changed in each text field the
    /// update gives.
    ///
    /// # Errors
    /// As [`Self::execute`].
    pub fn execute_noted(
        &self,
        update: &DraftUpdate,
        mut facts: Option<&mut OpFacts>,
    ) -> Result<Task, ProjectBoardError> {
        if let Some(facts) = facts.as_deref_mut() {
            let normalised = Normalised {
                title: update.title.as_deref().map(board_text::line_changes),
                description: update.description.as_deref().map(board_text::text_changes),
                approach: None,
                items: None,
            };
            // An edit that gives no text has nothing normalised to tell.
            let gives_text = normalised.title.is_some() || normalised.description.is_some();
            facts.normalised = gives_text.then_some(normalised);
        }
        let noting = facts.is_some();
        let mut read = None;
        let id = &update.id;
        let edit = DraftEdit {
            title: update.title.as_deref().map(board_text::line),
            description: update.description.as_deref().map(board_text::text),
            parent: update.parent.clone(),
            depends_on: update.depends_on.clone(),
            entry: update.entry,
        };
        let message = format!("board: edit draft {}", id.as_str());
        let noted = facts.as_deref_mut();
        let (store, clock) = (&*self.store, &*self.clock);
        let written = commit(store, clock, &message, noted, |board, when| {
            let task = task_on(board, id);
            if noting {
                // This attempt's read, or none: never an earlier one's.
                read = task.as_ref().ok().map(|task| task.fields().status);
            }
            let task = task?;
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
            Ok(vec![BoardArtifact::Task(edited)])
        });
        let written = written.map(written_task);
        if let Some(facts) = facts {
            facts.status_from = read;
            facts.status_to = written.as_ref().ok().map(|task| task.fields().status);
        }
        written
    }
}

#[cfg(test)]
#[path = "update_draft_task_tests.rs"]
mod tests;
