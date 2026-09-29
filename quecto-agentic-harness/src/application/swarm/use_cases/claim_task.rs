//! `Tasks.claim` (#2272): one member takes a ready task.
use std::sync::Arc;

use serde_json::Value;

use crate::application::swarm::board_operation::{detail, operation};
use crate::application::swarm::board_tasks::read_task;
use crate::application::swarm::dto::{ClaimTaskRequest, TaskRow};
use crate::application::swarm::ports::{BoardRepository, Clock, IdSource};
use crate::domain::swarm::{Access, BoardError};

/// Through the operation gate (a running run), in one transaction: a task
/// whose dependencies are not all `completed` is refused, then any task
/// that is not `ready`; only then is the claim token drawn (once per
/// successful claim, as Python draws `uuid4()` after its checks). The
/// task becomes `claimed` by the actor under the token, the event
/// `claimed{task,token}` names the task id as the caller gave it, and the
/// answer is the claimed task's dict, its token included. Two members
/// claiming at once serialise on the store's write lock: the second reads
/// the first's claim and is refused.
pub struct ClaimTask {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    ids: Arc<dyn IdSource>,
}

impl ClaimTask {
    pub fn new(
        repository: Arc<dyn BoardRepository>,
        clock: Arc<dyn Clock + Send + Sync>,
        ids: Arc<dyn IdSource>,
    ) -> Self {
        Self {
            repository,
            clock,
            ids,
        }
    }

    /// # Errors
    /// An authorisation or budget refusal, `unknown task`, `unmet
    /// dependencies`, `task is not ready to claim`, or the store's.
    pub fn execute(&self, request: ClaimTaskRequest) -> Result<TaskRow, BoardError> {
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
                let task = read_task(transaction, &request.task_id)?;
                for dependency in task.dependencies() {
                    let completed =
                        transaction.task_status(dependency)?.as_deref() == Some("completed");
                    if completed {
                        continue;
                    }
                    return Err(BoardError::new("unmet dependencies"));
                }
                match task.text("status") {
                    Some("ready") => {}
                    _ => return Err(BoardError::new("task is not ready to claim")),
                }
                let token = self.ids.hex32();
                debug_assert!(
                    token.len() == 32
                        && token
                            .bytes()
                            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f')),
                    "a claim token is uuid4().hex, 32 lowercase hex digits: {token}"
                );
                transaction.update_task_claim(&request.task_id, actor, &token)?;
                transaction.event(
                    actor,
                    self.clock.now_seconds(),
                    "claimed",
                    &detail([
                        ("task", request.task_id.clone()),
                        ("token", Value::from(token.as_str())),
                    ]),
                )?;
                let claimed = read_task(transaction, &request.task_id)?;
                debug_assert!(
                    claimed.text("status") == Some("claimed")
                        && claimed.text("owner") == Some(actor)
                        && claimed.text("token") == Some(token.as_str()),
                    "the task reads claimed by {actor} under the drawn token"
                );
                Ok(claimed)
            },
        )
    }
}

#[cfg(test)]
#[path = "claim_task_tests.rs"]
mod tests;
