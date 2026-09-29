//! `Tasks.task(task_id)` (#2272, #2277): the task inside a read-only
//! operation, with its owner's liveness (#1969).
use std::sync::Arc;

use super::OverRepository;
use crate::application::swarm::board_operation::operation;
use crate::application::swarm::board_read_models::with_owner_liveness;
use crate::application::swarm::board_tasks::read_task;
use crate::application::swarm::dto::{ReadTaskRequest, TaskRow};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError};

/// Through the operation gate as a read (`active=False, read_only=True`):
/// any member with a row may read, a dead one included, whatever the run's
/// status. The answer is the task's dict with its derived status and,
/// for a held claim, its owner's liveness (`board_read_models`).
pub struct ReadTask {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ReadTask {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation refusal, `unknown task`, or the store's.
    pub fn execute(&self, request: ReadTaskRequest) -> Result<TaskRow, BoardError> {
        let reading = Access {
            read_only: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            &request.actor,
            reading,
            |transaction, _| {
                let mut read = [read_task(transaction, &request.task_id)?];
                with_owner_liveness(transaction, &*self.clock, &mut read)?;
                let [task] = read;
                Ok(task)
            },
        )
    }
}

impl OverRepository for ReadTask {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "read_task_tests.rs"]
mod tests;
