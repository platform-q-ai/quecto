//! Puts a new task on the board: only what a plan sets ([`NewTask`]), a
//! draft or ready task with no claim, runs, reviews, pull requests or
//! items yet, whose parent and dependencies are already on the board (so
//! a new task never closes a dependency cycle), stamped with the time it
//! was created. Blocking: a lost write pauses the calling thread before
//! it is tried again.
use std::sync::Arc;

use super::board_change::{checked_writer, commit};
use crate::application::project_board::dto::{NewTask, ProjectBoardError};
use crate::application::project_board::ports::{BoardClock, BoardSnapshot, ProjectBoardStore};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::slug::Slug;
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
    pub fn execute(&self, new: &NewTask) -> Result<Task, ProjectBoardError> {
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
fn created_at(new: &NewTask, now: &Timestamp) -> TaskFields {
    let fields = TaskFields {
        id: new.id.clone(),
        title: new.title.clone(),
        kind: new.kind,
        description: new.description.clone(),
        status: new.status.into(),
        parent: new.parent.clone(),
        depends_on: new.depends_on.clone(),
        plan: new.plan.clone(),
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

/// The id is free (neither held nor set aside), and the parent and every
/// dependency are tasks on the board.
fn free(board: &BoardSnapshot, new: &NewTask) -> Result<(), ProjectBoardError> {
    let held = |id: &Slug| board.tasks.iter().any(|task| task.fields().id == *id);
    if held(&new.id) {
        return Err(ProjectBoardError::Exists(new.id.clone()));
    }
    if board
        .quarantined
        .iter()
        .any(|file| file.id.as_ref() == Some(&new.id))
    {
        return Err(ProjectBoardError::Quarantined(new.id.clone()));
    }
    match new
        .parent
        .iter()
        .chain(&new.depends_on)
        .find(|id| !held(id))
    {
        Some(missing) => Err(ProjectBoardError::UnknownReference(missing.clone())),
        None => Ok(()),
    }
}

#[cfg(test)]
#[path = "create_board_task_tests.rs"]
mod tests;
