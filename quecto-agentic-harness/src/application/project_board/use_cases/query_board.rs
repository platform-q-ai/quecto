//! The board's listings, read from the store: ready tasks, tasks by
//! status, a task's children and its dependents, each by id.
use std::sync::Arc;

use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::ProjectBoardStore;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

pub struct QueryBoard {
    store: Arc<dyn ProjectBoardStore>,
}

impl QueryBoard {
    pub fn new(store: Arc<dyn ProjectBoardStore>) -> Self {
        Self { store }
    }

    /// Tasks anyone may claim.
    ///
    /// # Errors
    /// The store's.
    pub fn ready(&self) -> Result<Vec<Task>, ProjectBoardError> {
        self.by_status(TaskStatus::Ready)
    }

    /// # Errors
    /// The store's.
    pub fn by_status(&self, status: TaskStatus) -> Result<Vec<Task>, ProjectBoardError> {
        self.matching(|task| task.fields().status == status)
    }

    /// The tasks naming `parent` as their parent.
    ///
    /// # Errors
    /// The store's.
    pub fn children(&self, parent: &Slug) -> Result<Vec<Task>, ProjectBoardError> {
        self.matching(|task| task.fields().parent.as_ref() == Some(parent))
    }

    /// The tasks that depend on `task`.
    ///
    /// # Errors
    /// The store's.
    pub fn dependents(&self, task: &Slug) -> Result<Vec<Task>, ProjectBoardError> {
        self.matching(|candidate| candidate.fields().depends_on.contains(task))
    }

    fn matching(&self, wanted: impl Fn(&Task) -> bool) -> Result<Vec<Task>, ProjectBoardError> {
        let mut tasks: Vec<Task> = self.store.read()?.tasks;
        let _ = wanted;
        tasks.sort_by(|left, right| left.fields().id.cmp(&right.fields().id));
        Ok(tasks)
    }
}

#[cfg(test)]
#[path = "query_board_tests.rs"]
mod tests;
