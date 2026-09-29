//! `Tasks.verify_task` (#2272): the coordinator completes submitted work.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use crate::application::swarm::board_operation::{detail, operation};
use crate::application::swarm::board_tasks::{read_task, stored_id};
use crate::application::swarm::dto::{
    TaskChange, TaskRow, TaskTransition, TaskUpdate, VerifyTaskRequest,
};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError, RefusalKind, python_equal};

/// Through the operation gate for the coordinator (a running run): the
/// task's token must equal the given one (by Python's `==`) and the task
/// be `submitted` or `completed`, and every evidence entry's revision must
/// equal the given one. Completed work is then left as it is; submitted
/// work is `completed`, the files reserved under that claim (and no other)
/// are deleted, and the event `verified{task,revision}` names the task id
/// and the revision as the caller gave them.
pub struct VerifyTask {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl VerifyTask {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation or budget refusal, `unknown task`, `stale claim or
    /// work not submitted`, `stale evidence revision`, stored evidence
    /// without revisions, or the store's.
    pub fn execute(&self, request: VerifyTaskRequest) -> Result<TaskChange, BoardError> {
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
                let changed = |transition| TaskChange {
                    task_id: stored_id(&task),
                    transition,
                };
                let current = task
                    .get("token")
                    .is_some_and(|token| python_equal(token, &request.token));
                let status = task.text("status");
                let reviewable = matches!(status, Some("submitted" | "completed"));
                let (true, true) = (current, reviewable) else {
                    return Err(BoardError::new(
                        RefusalKind::StaleToken,
                        "stale claim or work not submitted",
                    ));
                };
                current_revision(&task, &request.revision)?;
                if status == Some("completed") {
                    return Ok(changed(TaskTransition::Unchanged));
                }
                transaction.update_task_status(&request.task_id, &TaskUpdate::Complete)?;
                transaction.delete_claim_files(&request.task_id, &request.token)?;
                transaction.event(
                    actor,
                    self.clock.now_seconds(),
                    "verified",
                    &detail([
                        ("task", request.task_id.clone()),
                        ("revision", request.revision.clone()),
                    ]),
                )?;
                Ok(changed(TaskTransition::Applied))
            },
        )
    }
}

/// Every stored evidence entry's `revision` equals `revision`, read in
/// order until the first that does not, as Python's `any` reads them.
/// `submit` stores a list of dicts that each carry one; anything else is
/// found only in a file edited outside the board (the
/// `outside_edited_evidence` divergence) and is refused where Python
/// reaches it.
fn current_revision(task: &TaskRow, revision: &Value) -> Result<(), BoardError> {
    let unrevisioned = || {
        BoardError::new(
            RefusalKind::Store,
            "stored evidence is not a list of revisioned entries",
        )
    };
    let Some(Value::Array(entries)) = task.get("evidence") else {
        return Err(unrevisioned());
    };
    for entry in entries {
        let stored = entry
            .as_object()
            .and_then(|entry| entry.get("revision"))
            .ok_or_else(unrevisioned)?;
        if python_equal(stored, revision) {
            continue;
        }
        return Err(BoardError::new(
            RefusalKind::StaleRevision,
            "stale evidence revision",
        ));
    }
    Ok(())
}

impl OverRepository for VerifyTask {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "verify_task_tests.rs"]
mod tests;
