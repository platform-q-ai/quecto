//! `Tasks._task` inside a read-only operation (#2272): the raw task read.
//! S12 adds the owner liveness the member-facing `task` answers with.
use std::sync::Arc;

use crate::application::swarm::board_operation::operation;
use crate::application::swarm::board_tasks::read_task;
use crate::application::swarm::dto::{ReadTaskRequest, TaskRow};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError};

/// Through the operation gate as a read (`active=False, read_only=True`):
/// any member with a row may read, a dead one included, whatever the run's
/// status. The answer is the task's dict with its derived status.
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
            |transaction, _| read_task(transaction, &request.task_id),
        )
    }
}

#[cfg(test)]
#[path = "read_task_tests.rs"]
mod tests;
