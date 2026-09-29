//! `Tasks.submit` (#2272): the owner submits evidence for review.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use crate::application::swarm::board_operation::{detail, operation};
use crate::application::swarm::board_tasks::{owned, unsubmitted};
use crate::application::swarm::dto::{SubmitTaskRequest, TaskTransition, TaskUpdate};
use crate::application::swarm::ports::{BoardEncoding, BoardRepository, Clock};
use crate::domain::swarm::validation::TEXT_MAX_BYTES;
use crate::domain::swarm::{
    Access, BoardError, RefusalKind, bounded_text, python_equal, python_truthy,
};

/// The evidence (a nonempty list of dicts, each with a truthy `artifact`
/// and `revision`, its encoding bounded) is checked before the operation
/// gate (a running run). Only the claim's owner, with its current token,
/// submits: the same evidence again (by Python's `==`) on submitted work
/// is a no-op, other evidence is refused once submitted. The task is
/// `submitted` with the evidence as given, without blocker, and the event
/// `submitted{task}` names the task id as the caller gave it. Submission
/// is not completion: the coordinator verifies it.
pub struct SubmitTask {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    encoding: Arc<dyn BoardEncoding>,
}

impl SubmitTask {
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
    /// An evidence refusal, an authorisation or budget refusal, `unknown
    /// task`, `stale or unowned claim`, the submitted evidence's
    /// immutability, or the store's.
    pub fn execute(&self, request: SubmitTaskRequest) -> Result<TaskTransition, BoardError> {
        evidence(&request.evidence)?;
        bounded_text(
            &self.encoding.encode(&request.evidence)?,
            "evidence references",
            TEXT_MAX_BYTES,
        )?;
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
                let same_evidence = task
                    .get("evidence")
                    .is_some_and(|stored| python_equal(stored, &request.evidence));
                if task.text("status") == Some("submitted") && same_evidence {
                    return Ok(TaskTransition::Unchanged);
                }
                unsubmitted(&task)?;
                let update = TaskUpdate::Submit {
                    evidence: request.evidence.clone(),
                };
                transaction.update_task_status(&request.task_id, &update)?;
                transaction.event(
                    actor,
                    self.clock.now_seconds(),
                    "submitted",
                    &detail([("task", request.task_id.clone())]),
                )?;
                Ok(TaskTransition::Applied)
            },
        )
    }
}

/// A nonempty list of dicts whose `artifact` and `revision` are truthy by
/// Python's `bool` (a missing key is `None`).
fn evidence(value: &Value) -> Result<(), BoardError> {
    let truthy = |entry: &Value, key: &str| entry.get(key).is_some_and(python_truthy);
    let reference =
        |entry: &Value| entry.is_object() && truthy(entry, "artifact") && truthy(entry, "revision");
    match value {
        Value::Array(entries) if !entries.is_empty() && entries.iter().all(reference) => Ok(()),
        _ => Err(BoardError::new(
            RefusalKind::Invalid,
            "artifact and revision evidence required",
        )),
    }
}

impl OverRepository for SubmitTask {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
            encoding: self.encoding.clone(),
        }
    }
}

#[cfg(test)]
#[path = "submit_task_tests.rs"]
mod tests;
