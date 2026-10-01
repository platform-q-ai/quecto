//! `Tasks.release` (#2272): the owner gives its claim back.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use crate::application::swarm::board_operation::{detail, operation};
use crate::application::swarm::board_tasks::{owned, read_task};
use crate::application::swarm::dto::{ReleaseTaskRequest, TaskRow, TaskUpdate};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError};

/// Through the operation gate (a running run): only the claim's owner,
/// with its current token, releases it. The task is `ready` again without
/// owner, token or blocker, the files reserved under that claim (and no
/// other) are deleted, and the event `released{task}` names the task id as
/// the caller gave it.
pub struct ReleaseTask {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ReleaseTask {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation or budget refusal, `unknown task`, `stale or
    /// unowned claim`, or the store's.
    ///
    /// Answers the released task's row as it now stands (#2394), its id
    /// the one the row holds (#2303: the task the op acted on).
    pub fn execute(&self, request: ReleaseTaskRequest) -> Result<TaskRow, BoardError> {
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
                owned(transaction, &request.task_id, &request.token, actor)?;
                transaction.update_task_status(&request.task_id, &TaskUpdate::Release)?;
                transaction.delete_claim_files(&request.task_id, &request.token)?;
                transaction.event(
                    actor,
                    self.clock.now_seconds(),
                    "released",
                    &detail([("task", request.task_id.clone())]),
                )?;
                let released = read_task(transaction, &request.task_id)?;
                debug_assert!(
                    released.get("owner").is_some_and(Value::is_null)
                        && released.get("token").is_some_and(Value::is_null),
                    "a released task has no owner or token"
                );
                Ok(released)
            },
        )
    }
}

impl OverRepository for ReleaseTask {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "release_task_tests.rs"]
mod tests;
