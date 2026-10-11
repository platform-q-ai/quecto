//! The board's listings, read from the store's snapshot: ready tasks,
//! takeable tasks, tasks by status, a task's children and its dependents,
//! each by id, beside every file the board set aside. (S1e's index may
//! answer them faster later, behind this use case.)
use std::sync::Arc;

use crate::application::project_board::dto::{BoardListing, ProjectBoardError};
use crate::application::project_board::ports::ProjectBoardStore;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::claims::is_live;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

pub struct QueryBoard {
    store: Arc<dyn ProjectBoardStore>,
}

impl QueryBoard {
    pub fn new(store: Arc<dyn ProjectBoardStore>) -> Self {
        Self { store }
    }

    /// Ready tasks whose every dependency is on the board and done or
    /// archived (finished or cancelled: either way it no longer blocks):
    /// the ones anyone may start now. (`by_status(Ready)` lists them all.)
    ///
    /// # Errors
    /// The store's.
    pub fn ready(&self) -> Result<BoardListing, ProjectBoardError> {
        self.matching(|task, board| {
            let satisfied = |id: &Slug| {
                board.iter().any(|other| {
                    other.fields().id == *id
                        && matches!(
                            other.fields().status,
                            TaskStatus::Done | TaskStatus::Archived
                        )
                })
            };
            task.fields().status == TaskStatus::Ready
                && task.fields().depends_on.iter().all(satisfied)
        })
    }

    /// Held tasks whose claim expired by `now`: anyone may take them over.
    ///
    /// # Errors
    /// The store's.
    pub fn takeable(&self, now: &Timestamp) -> Result<BoardListing, ProjectBoardError> {
        self.matching(|task, _| {
            let fields = task.fields();
            fields.status.may_hold_claim()
                && fields
                    .claim
                    .as_ref()
                    .is_some_and(|claim| !is_live(claim, now))
        })
    }

    /// # Errors
    /// The store's.
    pub fn by_status(&self, status: TaskStatus) -> Result<BoardListing, ProjectBoardError> {
        self.matching(|task, _| task.fields().status == status)
    }

    /// The tasks naming `parent` as their parent.
    ///
    /// # Errors
    /// The store's.
    pub fn children(&self, parent: &Slug) -> Result<BoardListing, ProjectBoardError> {
        self.matching(|task, _| task.fields().parent.as_ref() == Some(parent))
    }

    /// The tasks that depend on `task`.
    ///
    /// # Errors
    /// The store's.
    pub fn dependents(&self, task: &Slug) -> Result<BoardListing, ProjectBoardError> {
        self.matching(|candidate, _| candidate.fields().depends_on.contains(task))
    }

    fn matching(
        &self,
        wanted: impl Fn(&Task, &[Task]) -> bool,
    ) -> Result<BoardListing, ProjectBoardError> {
        let board = self.store.read()?;
        let mut tasks: Vec<Task> = board
            .tasks
            .iter()
            .filter(|task| wanted(task, &board.tasks))
            .cloned()
            .collect();
        tasks.sort_by(|left, right| left.fields().id.cmp(&right.fields().id));
        Ok(BoardListing {
            head: crate::application::project_board::ports::BoardHead(String::new()),
            tasks,
            quarantined: board.quarantined,
        })
    }
}

#[cfg(test)]
#[path = "query_board_tests.rs"]
mod tests;
