//! `Tasks.revoke(task_id, reason)` (#2275, #1961): the coordinator takes a
//! claim back from an owner that will not finish, alive or not.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;

use crate::application::swarm::board_operation::{detail, operation, text};
use crate::application::swarm::board_recovery::{Notice, notify_revoked, reopen};
use crate::application::swarm::board_tasks::{HELD_CLAIM, read_task};
use crate::application::swarm::dto::{Revocation, RevokeTaskRequest, Revoked};
use crate::application::swarm::ports::{BoardEncoding, BoardRepository, Clock};
use crate::domain::swarm::validation::TEXT_MAX_BYTES;
use crate::domain::swarm::{Access, BoardError, RefusalKind, bounded};

/// The reason is bounded before the operation gate for the coordinator (a
/// running run). An unowned task that reads `ready` or `blocked` is
/// answered as it stands, and nothing is written. Otherwise the task must
/// be owned and hold a claim; it is reopened (`_reopen`), the event
/// `revoked{task,reason,previous_owner}` records the audit, the previous
/// owner is told when it can be (`_notify_revoked`), and the task is read
/// again as it now stands.
pub struct RevokeTask {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    encoding: Arc<dyn BoardEncoding>,
}

impl RevokeTask {
    pub fn new(
        repository: Arc<dyn BoardRepository>,
        clock: Arc<dyn Clock + Send + Sync>,
        encoding: Arc<dyn BoardEncoding>,
    ) -> Self {
        Self {
            repository,
            clock,
            encoding,
        }
    }

    /// # Errors
    /// The reason's bound, an authorisation or budget refusal, `unknown
    /// task`, work that is not owned and held, or the store's.
    pub fn execute(&self, request: RevokeTaskRequest) -> Result<Revoked, BoardError> {
        let reason = bounded(&request.reason, "revocation reason", TEXT_MAX_BYTES)?;
        let actor = request.actor.as_str();
        let coordinating = Access {
            active: true,
            coordinator: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            coordinating,
            |transaction, _| {
                let task = read_task(transaction, &request.task_id)?;
                let previous = task.get("owner").cloned().unwrap_or(Value::Null);
                let status = task.text("status");
                if previous.is_null() && matches!(status, Some("ready" | "blocked")) {
                    return Ok(Revoked {
                        task,
                        revocation: Revocation::Unowned,
                    });
                }
                let held = status.is_some_and(|status| HELD_CLAIM.contains(&status));
                if previous.is_null() || !held {
                    return Err(BoardError::new(
                        RefusalKind::WrongState,
                        "only claimed, blocked or submitted work can be revoked",
                    ));
                }
                reopen(transaction, &request.task_id)?;
                transaction.event(
                    actor,
                    self.clock.now_seconds(),
                    "revoked",
                    &detail([
                        ("task", request.task_id.clone()),
                        ("reason", text(reason)),
                        ("previous_owner", previous.clone()),
                    ]),
                )?;
                let notice = Notice {
                    actor,
                    clock: &*self.clock,
                    previous: &previous,
                    task_id: &request.task_id,
                    reason,
                };
                let notified = notify_revoked(transaction, &*self.encoding, &notice)?;
                Ok(Revoked {
                    task: read_task(transaction, &request.task_id)?,
                    revocation: Revocation::Revoked { notified },
                })
            },
        )
    }
}

impl OverRepository for RevokeTask {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
            encoding: self.encoding.clone(),
        }
    }
}

#[cfg(test)]
#[path = "revoke_task_tests.rs"]
mod tests;
