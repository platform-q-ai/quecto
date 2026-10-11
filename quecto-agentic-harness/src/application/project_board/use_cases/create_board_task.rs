//! Puts a new task on the board: only what a plan sets ([`NewBoardTask`]), a
//! draft or ready task with no claim, runs, reviews, pull requests or
//! items yet, whose parent and dependencies are already on the board,
//! stamped with the time it was created. It never closes a cycle: not
//! through new references, which must exist, nor through a task still
//! naming the id of one that was deleted. Blocking: a lost write pauses the calling thread before
//! it is tried again.
use std::sync::Arc;

use super::board_change::{checked_writer, commit};
use super::line_ends::{lf, plan_lf};
use super::references::{References, references_hold};
use crate::application::project_board::dto::{NewBoardTask, ProjectBoardError};
use crate::application::project_board::ports::{BoardClock, BoardSnapshot, ProjectBoardStore};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

pub struct CreateBoardTask {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
}

impl CreateBoardTask {
    /// # Errors
    /// The store writes as an invalid identity.
    pub fn new(
        store: Arc<dyn ProjectBoardStore>,
        clock: Arc<dyn BoardClock>,
    ) -> Result<Self, ProjectBoardError> {
        checked_writer(&*store)?;
        Ok(Self { store, clock })
    }

    /// The task `new` describes, created and updated now.
    ///
    /// # Errors
    /// A parent or dependency not on the board, or set aside; an id a
    /// task already names; the schema's refusal; an id the board holds or
    /// has set aside; the clock's; or the store's.
    pub fn execute(&self, new: &NewBoardTask) -> Result<Task, ProjectBoardError> {
        let message = format!("board: create {}", new.id.as_str());
        let tasks = commit(&*self.store, &*self.clock, &message, |board, when| {
            free(board, new)?;
            Ok(vec![
                Task::new(created_at(new, &when.now)).map_err(ProjectBoardError::Invalid)?,
            ])
        })?;
        Ok(tasks.into_iter().next().expect("a create writes its task"))
    }
}

/// The new task's fields, created and updated `now`, with no claim, runs,
/// reviews, pull requests or items yet.
fn created_at(new: &NewBoardTask, now: &Timestamp) -> TaskFields {
    let fields = TaskFields {
        id: new.id.clone(),
        title: lf(&new.title),
        kind: new.kind,
        description: lf(&new.description),
        status: new.status.into(),
        parent: new.parent.clone(),
        depends_on: new.depends_on.clone(),
        plan: plan_lf(&new.plan),
        items: vec![],
        team: new.team.clone(),
        claim: None,
        runs: vec![],
        reviews: vec![],
        prs: vec![],
        created: now.clone(),
        updated: now.clone(),
    };
    assert!(
        matches!(fields.status, TaskStatus::Draft | TaskStatus::Ready),
        "a task is created a draft or ready"
    );
    fields
}

/// The id is free (neither held nor set aside), and the references hold
/// ([`references_hold`]).
fn free(board: &BoardSnapshot, new: &NewBoardTask) -> Result<(), ProjectBoardError> {
    let held = board.tasks.iter().any(|task| task.fields().id == new.id);
    let set_aside = board
        .quarantined
        .iter()
        .any(|file| file.id.as_ref() == Some(&new.id));
    match (held, set_aside) {
        (true, _) => Err(ProjectBoardError::Exists(new.id.clone())),
        (false, true) => Err(ProjectBoardError::Quarantined(new.id.clone())),
        (false, false) => references_hold(
            board,
            &References {
                id: &new.id,
                parent: new.parent.as_ref(),
                depends_on: &new.depends_on,
            },
        ),
    }
}

#[cfg(test)]
#[path = "create_board_task_tests.rs"]
mod tests;
