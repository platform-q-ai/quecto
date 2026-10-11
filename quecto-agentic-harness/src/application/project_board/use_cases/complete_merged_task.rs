//! The system's move behind "a merged PR moves its task to done" (epic
//! #2481): a held task whose linked pull requests have all merged is done,
//! whoever holds it. Not an agent's operation: S2 wires it from the
//! repository's merge events only, and the merged set it is given is what
//! those events report.
use std::collections::BTreeSet;
use std::sync::Arc;

use super::board_change::{commit, task_on};
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{BoardClock, ProjectBoardStore};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::transitions::complete_merged;
use crate::domain::project_board::value_objects::slug::Slug;

pub struct CompleteMergedTask {
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
}

impl CompleteMergedTask {
    pub fn new(store: Arc<dyn ProjectBoardStore>, clock: Arc<dyn BoardClock>) -> Self {
        Self { store, clock }
    }

    /// Task `id` done, when `merged` holds every pull request it links.
    ///
    /// # Errors
    /// As [`ChangeBoardTask`](super::ChangeBoardTask)'s.
    pub fn execute(&self, id: &Slug, merged: &BTreeSet<u64>) -> Result<Task, ProjectBoardError> {
        let message = format!("board: {} done, its pull requests merged", id.as_str());
        let (tasks, _) = commit(&*self.store, &*self.clock, &message, |board, when| {
            let task = task_on(board, id)?;
            when.may_build_on(task)?;
            Ok(vec![
                complete_merged(task, merged, &when.now).map_err(ProjectBoardError::Rule)?,
            ])
        })?;
        Ok(tasks.into_iter().next().expect("a change writes its task"))
    }
}
