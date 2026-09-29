//! `Tasks.tasks(offset=0, limit=50)` (#2277, #1969): a page of the tasks
//! by id, each with its owner's liveness.
use std::sync::Arc;

use super::OverRepository;
use crate::application::swarm::board_operation::operation;
use crate::application::swarm::board_read_models::{page_bounds, reading, with_owner_liveness};
use crate::application::swarm::board_tasks::read_task;
use crate::application::swarm::dto::{ListTasksRequest, TaskPage};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{BoardError, RefusalKind};

/// The offset must be an integer of at least 0 and the limit an integer
/// from 1 to 100, checked before the gate. Through the read-only gate,
/// each task of the page as `_task` reads it, then the owners' liveness in
/// one grouped scan for the page.
pub struct ListTasks {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ListTasks {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// The page's bounds, an authorisation refusal, or the store's.
    pub fn execute(&self, request: ListTasksRequest) -> Result<TaskPage, BoardError> {
        let Some((offset, limit)) = page_bounds(&request.offset, &request.limit) else {
            return Err(BoardError::new(
                RefusalKind::Invalid,
                "task page requires nonnegative offset and limit 1 through 100",
            ));
        };
        let clock = &*self.clock;
        operation(
            &*self.repository,
            clock,
            &request.actor,
            reading(),
            |transaction, _| {
                let mut page = transaction
                    .task_ids(offset, limit)?
                    .iter()
                    .map(|id| read_task(transaction, id))
                    .collect::<Result<Vec<_>, _>>()?;
                let owners_scanned = with_owner_liveness(transaction, clock, &mut page)?;
                Ok(TaskPage {
                    tasks: page,
                    owners_scanned,
                })
            },
        )
    }
}

impl OverRepository for ListTasks {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "list_tasks_tests.rs"]
mod tests;
