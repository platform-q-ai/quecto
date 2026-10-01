//! `Tasks.unblock` (#2272): the owner resumes its blocked claim.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use crate::application::swarm::board_operation::{detail, operation};
use crate::application::swarm::board_tasks::{changed, owned};
use crate::application::swarm::dto::{TaskChange, TaskTransition, TaskUpdate, UnblockTaskRequest};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::validation::TEXT_MAX_BYTES;
use crate::domain::swarm::{Access, BoardError, RefusalKind, bounded};

/// The resolution is bounded before the operation gate (a running run).
/// Only the claim's owner, with its current token, resumes it: a claimed
/// task is left as it is; a blocked one is `claimed` again under the same
/// token, without blocker, and the event `unblocked{task,reason}` names
/// the task id as the caller gave it. Submitted work does not resume.
pub struct UnblockTask {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl UnblockTask {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// The resolution's bound, an authorisation or budget refusal,
    /// `unknown task`, `stale or unowned claim`, `only blocked or claimed
    /// work may resume`, or the store's.
    pub fn execute(&self, request: UnblockTaskRequest) -> Result<TaskChange, BoardError> {
        let reason = bounded(&request.reason, "resolution", TEXT_MAX_BYTES)?;
        let actor = request.actor.as_str();
        let running = Access {
            active: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            running,
            |transaction, _| {
                let task = owned(transaction, &request.task_id, &request.token, actor)?;
                match task.text("status") {
                    Some("claimed") => Ok(TaskChange {
                        task,
                        transition: TaskTransition::Unchanged,
                    }),
                    Some("blocked") => {
                        transaction.update_task_status(&request.task_id, &TaskUpdate::Unblock)?;
                        transaction.event(
                            actor,
                            self.clock.now_seconds(),
                            "unblocked",
                            &detail([
                                ("task", request.task_id.clone()),
                                ("reason", Value::from(reason)),
                            ]),
                        )?;
                        changed(transaction, &request.task_id, "claimed")
                    }
                    _ => Err(BoardError::new(
                        RefusalKind::WrongState,
                        "only blocked or claimed work may resume",
                    )),
                }
            },
        )
    }
}

impl OverRepository for UnblockTask {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "unblock_task_tests.rs"]
mod tests;
