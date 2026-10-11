//! Puts a new task on the board: as a draft or ready, unclaimed, stamped
//! with the time it was created.
use std::sync::Arc;

use super::board_change::commit;
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{BoardClock, ProjectBoardStore};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

pub struct CreateBoardTask {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
}

impl CreateBoardTask {
    pub fn new(store: Arc<dyn ProjectBoardStore>, clock: Arc<dyn BoardClock>) -> Self {
        Self { store, clock }
    }

    /// The task `fields` describe, created and updated now, unless the
    /// board already holds its id.
    ///
    /// # Errors
    /// A status other than draft or ready, a claim, the schema's refusal,
    /// an id already taken, the clock's, or the store's.
    pub fn execute(&self, fields: TaskFields) -> Result<Task, ProjectBoardError> {
        let creatable = matches!(fields.status, TaskStatus::Draft | TaskStatus::Ready);
        if !creatable || fields.claim.is_some() {
            return Err(ProjectBoardError::Rule(BoardRuleError::NotAllowed {
                from: TaskStatus::Draft,
                to: fields.status,
            }));
        }
        let message = format!("board: create {}", fields.id.as_str());
        let (tasks, _) = commit(&*self.store, &*self.clock, &message, |board, now| {
            if board.tasks.iter().any(|task| task.fields().id == fields.id) {
                return Err(ProjectBoardError::Exists(fields.id.clone()));
            }
            let mut created = fields.clone();
            (created.created, created.updated) = (now.clone(), now.clone());
            Ok(vec![
                Task::new(created).map_err(ProjectBoardError::Invalid)?,
            ])
        })?;
        Ok(tasks.into_iter().next().expect("a create writes its task"))
    }
}

#[cfg(test)]
#[path = "create_board_task_tests.rs"]
mod tests;
