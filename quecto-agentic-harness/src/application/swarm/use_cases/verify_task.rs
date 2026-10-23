//! `Tasks.verify_task` (#2272): the coordinator completes submitted work.
use std::sync::Arc;

use serde_json::Value;

use super::OverRepository;
use crate::application::swarm::dto::{
    TaskChange, TaskRow, TaskTransition, TaskUpdate, VerifyTaskRequest,
};
use crate::application::swarm::ports::{
    board_files::{Ancestry, CheckoutHistory},
    board_repository::BoardRepository,
    lifecycle::Clock,
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
/// revision`; the history traces why it declined. Only the
/// coordinator verifies, so only it carries evidence; the worker's
/// submission stays immutable to the worker.
///
/// git is never asked inside a transaction (#2691 review M1): the first
/// operation finds the carry and writes nothing, the history is asked
/// with no transaction open, and a second operation writes the carry only
/// if the claim token, the `submitted` status and the stored evidence are
/// still what the first found; otherwise `stale evidence revision`.
pub struct VerifyTask {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
    history: Arc<dyn CheckoutHistory>,
}

/// A carry the first operation found: the submission's evidence as it
/// read it, and that evidence carried to the verified commit.
struct CarryPlan {
    previous: Value,
    carry: EvidenceCarry,
}

/// What one operation of a verification did: verified (or found verified
/// work), or found a carry to ask the history about, writing nothing.
enum Verified {
    Done(TaskChange),
    Carry(CarryPlan),
}

fn stale() -> BoardError {
    BoardError::new(RefusalKind::StaleRevision, "stale evidence revision")
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

    /// # Errors
    /// An authorisation or budget refusal, `unknown task`, `stale claim or
    /// work not submitted`, `stale evidence revision`, stored evidence
    /// without revisions, or the store's.
    pub fn execute(&self, request: VerifyTaskRequest) -> Result<TaskChange, BoardError> {
        let plan = match self.verify(&request, None)? {
            Verified::Done(change) => return Ok(change),
            Verified::Carry(plan) => plan,
        };
        let target = request
            .revision
            .as_str()
            .expect("a carry is found only for a full commit id revision");
        // The history traces why it declines; the refusal is the stale one.
        if self.history.ancestry(&plan.carry.sources, target) != Ancestry::Confirmed {
            return Err(stale());
        }
        match self.verify(&request, Some(&plan))? {
            Verified::Done(change) => Ok(change),
            Verified::Carry(_) => unreachable!("a confirmed carry is written or refused"),
        }
    }

    /// One operation of a verification. Without `confirmed`, a carry found
    /// is answered unwritten; with it, the carry is written only when the
    /// stored evidence is still the evidence `confirmed` was found over.
    fn verify(
        &self,
        request: &VerifyTaskRequest,
        confirmed: Option<&CarryPlan>,
    ) -> Result<Verified, BoardError> {
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
                    Err(refusal) => {
                        let found = carry(&task, &request.revision, refusal)?;
                        match confirmed {
                            None => return Ok(Verified::Carry(found)),
                            Some(plan) if python_equal(&found.previous, &plan.previous) => {
                                Some(found)
                            }
                            // The submission changed since the history was
                            // asked about it.
                            Some(_) => return Err(stale()),
                        }
                    }
                };
                if status == Some("completed") {
                    assert!(carried.is_none(), "only submitted work is carried");
                    return Ok(Verified::Done(TaskChange {
                        task,
                        transition: TaskTransition::Unchanged,
                    }));
                }
                let (task_id, revision) = (request.task_id.clone(), request.revision.clone());
                let verified = match carried {
                    None => detail([("task", task_id), ("revision", revision)]),
                    Some(CarryPlan { previous, carry }) => {
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
                changed(transaction, &request.task_id, "completed").map(Verified::Done)
            },
        )
    }
}

/// The carry of a submission's stale evidence to `revision` (#2687), with
/// the evidence as submitted, when the policy admits it: submitted work,
/// a stale revision, and full commit ids throughout. Otherwise `refusal`,
/// the one it answers.
fn carry(task: &TaskRow, revision: &Value, refusal: BoardError) -> Result<CarryPlan, BoardError> {
    let submitted = task.text("status") == Some("submitted");
    let stored = task.get("evidence");
    let found = stored.and_then(|evidence| evidence_carry(evidence, revision));
    match (submitted, refusal.kind(), stored, found) {
        (true, RefusalKind::StaleRevision, Some(stored), Some(carry)) => Ok(CarryPlan {
            previous: stored.clone(),
            carry,
        }),
        _ => Err(refusal),
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
        return Err(stale());
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
