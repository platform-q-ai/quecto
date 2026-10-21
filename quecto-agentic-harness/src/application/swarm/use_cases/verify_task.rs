//! `Tasks.verify_task` (#2272): the coordinator completes submitted work.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use crate::application::swarm::dto::{
    TaskChange, TaskRow, TaskTransition, TaskUpdate, VerifyTaskRequest,
};
use crate::application::swarm::ports::{
    board_files::CheckoutHistory, board_repository::BoardRepository, lifecycle::Clock,
};
use crate::application::swarm::services::board_operation::{detail, operation};
use crate::application::swarm::services::board_tasks::{changed, read_task};
use crate::domain::swarm::{
    events::telemetry::RefusalKind,
    services::evidence_carry::{EvidenceCarry, evidence_carry},
    services::policy::Access,
    value_objects::board_error::BoardError,
    value_objects::python_value::python_equal,
};

/// Through the operation gate for the coordinator (a running run): the
/// task's token must equal the given one (by Python's `==`) and the task
/// be `submitted` or `completed`, and every evidence entry's revision must
/// equal the given one. Completed work is then left as it is; submitted
/// work is `completed`, the files reserved under that claim (and no other)
/// are deleted, and the event `verified{task,revision}` names the task id
/// and the revision as the caller gave them.
///
/// The coordinator commits a worker's change, so it verifies at its own
/// commit while the submission names the commit the change was made on
/// (#2687). Submitted work whose evidence is stale at `revision` then has
/// its evidence carried to it, when every revision involved is a full
/// commit id and the checkout's history confirms each earlier one is an
/// ancestor of `revision`: the evidence is stored at `revision`, and the
/// event adds `carried_from` (the earlier commits) and `previous_evidence`
/// (the submission as made). Anything else stays `stale evidence
/// revision`. Only the coordinator verifies, so only it carries evidence;
/// the worker's submission stays immutable to the worker.
pub struct VerifyTask {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    history: Arc<dyn CheckoutHistory>,
}

impl VerifyTask {
    pub fn new(
        repository: Arc<dyn BoardRepository>,
        clock: Arc<dyn Clock + Send + Sync>,
        history: Arc<dyn CheckoutHistory>,
    ) -> Self {
        Self {
            repository,
            clock,
            history,
        }
    }

    /// The carry of a submission's stale evidence to `revision` (#2687),
    /// with the evidence as submitted, when the policy admits it and the
    /// checkout's history confirms every earlier commit; otherwise
    /// `stale`, the refusal it answers.
    fn carry(
        &self,
        task: &TaskRow,
        revision: &Value,
        stale: BoardError,
    ) -> Result<(EvidenceCarry, Value), BoardError> {
        let submitted = task.text("status") == Some("submitted");
        let stored = task.get("evidence");
        let carry = stored.and_then(|evidence| evidence_carry(evidence, revision));
        match (submitted, stale.kind(), stored, carry, revision.as_str()) {
            (true, RefusalKind::StaleRevision, Some(stored), Some(carry), Some(target))
                if carry
                    .sources
                    .iter()
                    .all(|source| self.history.is_ancestor(source, target)) =>
            {
                Ok((carry, stored.clone()))
            }
            _ => Err(stale),
        }
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
                let carried = match current_revision(&task, &request.revision) {
                    Ok(()) => None,
                    Err(stale) => Some(self.carry(&task, &request.revision, stale)?),
                };
                if status == Some("completed") {
                    assert!(carried.is_none(), "only submitted work is carried");
                    return Ok(TaskChange {
                        task,
                        transition: TaskTransition::Unchanged,
                    });
                }
                let (task_id, revision) = (request.task_id.clone(), request.revision.clone());
                let verified = match carried {
                    None => detail([("task", task_id), ("revision", revision)]),
                    Some((carry, previous)) => {
                        transaction.replace_task_evidence(&request.task_id, &carry.evidence)?;
                        detail([
                            ("task", task_id),
                            ("revision", revision),
                            ("carried_from", Value::from(carry.sources)),
                            ("previous_evidence", previous),
                        ])
                    }
                };
                transaction.update_task_status(&request.task_id, &TaskUpdate::Complete)?;
                transaction.delete_claim_files(&request.task_id, &request.token)?;
                transaction.event(actor, self.clock.now_seconds(), "verified", &verified)?;
                changed(transaction, &request.task_id, "completed")
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
            history: self.history.clone(),
        }
    }
}

#[cfg(test)]
#[path = "verify_task_tests.rs"]
mod tests;
