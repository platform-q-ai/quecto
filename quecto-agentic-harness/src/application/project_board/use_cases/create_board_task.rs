//! Puts a new task on the board: only the task's own fields
//! ([`NewBoardTask`]), a draft or ready task with no plan, team, claim,
//! runs, reviews, pull requests or task items yet, whose parent and
//! dependencies are already on the board, stamped with the time it was
//! created. It never closes a cycle: not through new references, which
//! must exist, nor through a task still naming the id of one that was
//! deleted. Blocking: a lost write pauses the calling thread before it is
//! tried again.
use std::sync::Arc;

use super::board_change::{checked_writer, commit, written_task};
use super::board_view;
use super::description_cells::held;
use super::task_pointers::{Pointers, pointers_hold};
use crate::application::project_board::dto::{NewBoardTask, ProjectBoardError};
use crate::application::project_board::ports::{
    BoardArtifact, BoardClock, BoardSnapshot, BoardTextCells, ProjectBoardStore,
};
use crate::application::project_board::telemetry::OpFacts;
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::entities::task_parts::DescriptionForm;
use crate::domain::project_board::value_objects::board_text;
use crate::domain::project_board::value_objects::telemetry::Normalised;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

pub struct CreateBoardTask {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    /// What a board file holds: a sectioned description's outcomes and
    /// links are checked against it as they enter (#2705 T2).
    cells: Arc<dyn BoardTextCells>,
}

impl CreateBoardTask {
    /// # Errors
    /// The store writes as an invalid identity.
    pub fn new(
        store: Arc<dyn ProjectBoardStore>,
        clock: Arc<dyn BoardClock>,
        cells: Arc<dyn BoardTextCells>,
    ) -> Result<Self, ProjectBoardError> {
        checked_writer(&*store)?;
        Ok(Self {
            store,
            clock,
            cells,
        })
    }

    /// The task `new` describes, created and updated now.
    ///
    /// # Errors
    /// A parent or dependency not on the board, or set aside; an id a
    /// task already names; the schema's refusal; an id the board holds or
    /// has set aside; an id a plan, run, change, review or discussion
    /// already names, or while a plan, run, change or review of a task by
    /// that id, or any discussion, is set aside; the clock's; or the
    /// store's.
    pub fn execute(&self, new: &NewBoardTask) -> Result<Task, ProjectBoardError> {
        self.execute_noted(new, None)
    }

    /// [`Self::execute`], noting in `facts` (epic #2481 T4), when given,
    /// the write's attempts, the status of the task it wrote, and what the
    /// normaliser changed in its title and in its description, when one
    /// was given.
    ///
    /// # Errors
    /// As [`Self::execute`].
    pub fn execute_noted(
        &self,
        new: &NewBoardTask,
        mut facts: Option<&mut OpFacts>,
    ) -> Result<Task, ProjectBoardError> {
        if let Some(facts) = facts.as_deref_mut() {
            facts.normalised = Some(Normalised {
                title: Some(board_text::line_changes(&new.title)),
                // A request that gave no description holds an empty one,
                // which no normaliser changes: none is told.
                description: (!new.description.is_empty())
                    .then(|| board_text::text_changes(&new.description)),
                approach: None,
                items: None,
            });
        }
        let message = format!("board: create {}", new.id.as_str());
        let (store, clock) = (&*self.store, &*self.clock);
        let noted = facts.as_deref_mut();
        let written = commit(store, clock, &message, noted, |board, when| {
            free(board, new)?;
            let task = Task::entered(created_at(new, &when.now), new.entry)
                .map_err(ProjectBoardError::Invalid)?;
            held(&task.fields().description, new.entry, &*self.cells)
                .map_err(ProjectBoardError::Invalid)?;
            Ok(vec![BoardArtifact::Task(task)])
        });
        let written = written.map(written_task);
        if let Some(facts) = facts {
            facts.status_to = written.as_ref().ok().map(|task| task.fields().status);
        }
        written
    }
}

/// The new task's fields, created and updated `now`, with no plan, team,
/// claim, runs, reviews, pull requests or task items yet.
fn created_at(new: &NewBoardTask, now: &Timestamp) -> TaskFields {
    let fields = TaskFields {
        id: new.id.clone(),
        title: board_text::line(&new.title),
        kind: new.kind,
        description: board_text::text(&new.description),
        // Task::entered keeps it in the form its entry gives.
        description_form: DescriptionForm::Legacy,
        status: new.status.into(),
        parent: new.parent.clone(),
        depends_on: new.depends_on.clone(),
        accepted_plan: None,
        claim: None,
        prs: vec![],
        github_issue: None,
        resolution: None,
        superseded_by: vec![],
        references: vec![],
        created: now.clone(),
        updated: now.clone(),
    };
    assert!(
        matches!(fields.status, TaskStatus::Draft | TaskStatus::Ready),
        "a task is created a draft or ready"
    );
    fields
}

/// The id is free (neither held nor set aside, and named by no other
/// artifact: a new task would take over what it says), and the references
/// hold ([`pointers_hold`]). While a file that may name the id is set
/// aside ([`board_view::naming`]: one of that task's, or any discussion)
/// the create waits for it: creates are rare.
fn free(board: &BoardSnapshot, new: &NewBoardTask) -> Result<(), ProjectBoardError> {
    match board_view::task(board, &new.id) {
        Ok(_) => Err(ProjectBoardError::Exists(new.id.clone())),
        Err(ProjectBoardError::NotFound(_)) => match board_view::naming(board, &new.id)? {
            Some(by) => Err(ProjectBoardError::IdInUse {
                id: new.id.clone(),
                by,
            }),
            None => pointers_hold(
                board,
                &Pointers {
                    id: &new.id,
                    parent: new.parent.as_ref(),
                    depends_on: &new.depends_on,
                },
            ),
        },
        Err(refused) => Err(refused),
    }
}

#[cfg(test)]
#[path = "create_board_task_tests.rs"]
mod tests;
