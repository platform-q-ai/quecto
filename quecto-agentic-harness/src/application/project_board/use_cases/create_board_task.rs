//! Puts a new task on the board: an unclaimed draft or ready task with no
//! pull requests, runs or reviews yet, whose parent and dependencies are
//! already on the board (so a new task never closes a dependency cycle),
//! stamped with the time it was created.
use std::sync::Arc;

use super::board_change::commit;
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{BoardClock, BoardSnapshot, ProjectBoardStore};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

pub struct CreateBoardTask {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
}

impl CreateBoardTask {
    pub fn new(store: Arc<dyn ProjectBoardStore>, clock: Arc<dyn BoardClock>) -> Self {
        Self { store, clock }
    }

    /// The task `fields` describe, created and updated now.
    ///
    /// # Errors
    /// Fields a new task may not have, a parent or dependency not on the
    /// board, the schema's refusal, an id the board holds or has set
    /// aside, the clock's, or the store's.
    pub fn execute(&self, fields: TaskFields) -> Result<Task, ProjectBoardError> {
        creatable(&fields)?;
        let message = format!("board: create {}", fields.id.as_str());
        let (tasks, _) = commit(&*self.store, &*self.clock, &message, |board, when| {
            free(board, &fields)?;
            let mut created = fields.clone();
            (created.created, created.updated) = (when.now.clone(), when.now.clone());
            Ok(vec![
                Task::new(created).map_err(ProjectBoardError::Invalid)?,
            ])
        })?;
        Ok(tasks.into_iter().next().expect("a create writes its task"))
    }
}

fn creatable(fields: &TaskFields) -> Result<(), ProjectBoardError> {
    let refused = |why: &str| Err(ProjectBoardError::NotCreatable(why.into()));
    match () {
        _ if !matches!(fields.status, TaskStatus::Draft | TaskStatus::Ready) => {
            refused("is a draft or ready")
        }
        _ if fields.claim.is_some() => refused("has no claim"),
        _ if !fields.prs.is_empty() => refused("links no pull request"),
        _ if !fields.runs.is_empty() => refused("has no runs"),
        _ if !fields.reviews.is_empty() => refused("has no reviews"),
        _ => Ok(()),
    }
}

/// The id is free (neither held nor set aside), and the parent and every
/// dependency are tasks on the board.
fn free(board: &BoardSnapshot, fields: &TaskFields) -> Result<(), ProjectBoardError> {
    let held = |id: &Slug| board.tasks.iter().any(|task| task.fields().id == *id);
    if held(&fields.id) {
        return Err(ProjectBoardError::Exists(fields.id.clone()));
    }
    if board
        .quarantined
        .iter()
        .any(|file| file.id.as_ref() == Some(&fields.id))
    {
        return Err(ProjectBoardError::Quarantined(fields.id.clone()));
    }
    match fields
        .parent
        .iter()
        .chain(&fields.depends_on)
        .find(|id| !held(id))
    {
        Some(missing) => Err(ProjectBoardError::UnknownReference(missing.clone())),
        None => Ok(()),
    }
}

#[cfg(test)]
#[path = "create_board_task_tests.rs"]
mod tests;
