//! `Tasks.recover(task_id, release_files=False)` (#2275, #1961): the
//! coordinator reopens work whose owner's death the harness confirmed.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;

use crate::application::swarm::board_operation::{detail, operation};
use crate::application::swarm::board_recovery::reopen;
use crate::application::swarm::board_tasks::{HELD_CLAIM, read_task};
use crate::application::swarm::dto::{RecoverTaskRequest, Recovered};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError, RefusalKind};

/// Through the operation gate for the coordinator (a running run): the
/// task must hold a claim (`claimed`, `blocked` or `submitted`), and its
/// owner's status must be the text `dead` (only the status is read, as
/// Python's `SELECT status FROM members WHERE id=?` reads it).
/// Reservations the task still holds (an abrupt exit retains them) are
/// freed only when `release_files` is exactly `true` (Python's `is
/// True`). The task is reopened (`_reopen`) and the event `recovered{task,reservations_released}` names the task
/// id as given and how many reservations went.
pub struct RecoverTask {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl RecoverTask {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation or budget refusal, `unknown task`, work that holds
    /// no claim, an owner not confirmed dead, retained reservations
    /// without `release_files=True`, or the store's.
    pub fn execute(&self, request: RecoverTaskRequest) -> Result<Recovered, BoardError> {
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
                let held = task
                    .text("status")
                    .is_some_and(|status| HELD_CLAIM.contains(&status));
                if !held {
                    return Err(BoardError::new(
                        RefusalKind::WrongState,
                        "only abandoned active work can be recovered",
                    ));
                }
                let owner = task.get("owner").cloned().unwrap_or(Value::Null);
                let dead = transaction
                    .member_status(&owner)?
                    .is_some_and(|row| row.status.as_deref() == Some("dead"));
                if !dead {
                    return Err(BoardError::new(
                        RefusalKind::WrongState,
                        "recovery requires confirmed worker death; revoke(id, reason) reassigns a live owner",
                    ));
                }
                let retained = transaction.task_file_count(&request.task_id)?;
                debug_assert!(retained >= 0, "a count is never negative");
                let release = request.release_files == Value::Bool(true);
                if retained != 0 && !release {
                    return Err(BoardError::new(
                        RefusalKind::WrongState,
                        "reservations retained after an abrupt exit; recover(id, release_files=True) frees them, or revoke(id, reason)",
                    ));
                }
                reopen(transaction, &request.task_id)?;
                transaction.event(
                    actor,
                    self.clock.now_seconds(),
                    "recovered",
                    &detail([
                        ("task", request.task_id.clone()),
                        ("reservations_released", Value::from(retained)),
                    ]),
                )?;
                Ok(Recovered {
                    task: read_task(transaction, &request.task_id)?,
                    reservations_released: retained,
                })
            },
        )
    }
}

impl OverRepository for RecoverTask {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "recover_task_tests.rs"]
mod tests;
