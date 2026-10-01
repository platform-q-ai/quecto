//! `Tasks.block` (#2272): the owner marks its claim blocked.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use crate::application::swarm::board_operation::{detail, operation};
use crate::application::swarm::board_tasks::{changed, owned, unsubmitted};
use crate::application::swarm::dto::{BlockTaskRequest, TaskChange, TaskTransition, TaskUpdate};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::validation::TEXT_MAX_BYTES;
use crate::domain::swarm::{Access, BoardError, bounded, python_equal};

/// The reason is bounded before the operation gate (a running run). Only
/// the claim's owner, with its current token, blocks it, and never once
/// its evidence is submitted. A task already blocked by the same reason
/// is left as it is; otherwise it is `blocked` by the reason, and the
/// event `blocked{task,reason}` names the task id as the caller gave it.
pub struct BlockTask {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl BlockTask {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// The reason's bound, an authorisation or budget refusal, `unknown
    /// task`, `stale or unowned claim`, the submitted evidence's
    /// immutability, or the store's.
    pub fn execute(&self, request: BlockTaskRequest) -> Result<TaskChange, BoardError> {
        let reason = bounded(&request.reason, "blocker", TEXT_MAX_BYTES)?;
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
                unsubmitted(&task)?;
                let same_blocker = task
                    .get("blocker")
                    .is_some_and(|blocker| python_equal(blocker, &request.reason));
                if task.text("status") == Some("blocked") && same_blocker {
                    return Ok(TaskChange {
                        task,
                        transition: TaskTransition::Unchanged,
                    });
                }
                let update = TaskUpdate::Block {
                    reason: reason.to_owned(),
                };
                transaction.update_task_status(&request.task_id, &update)?;
                transaction.event(
                    actor,
                    self.clock.now_seconds(),
                    "blocked",
                    &detail([
                        ("task", request.task_id.clone()),
                        ("reason", Value::from(reason)),
                    ]),
                )?;
                changed(transaction, &request.task_id, "blocked")
            },
        )
    }
}

impl OverRepository for BlockTask {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "block_task_tests.rs"]
mod tests;
