//! The completion methods of the board dispatch (#2273): `complete`,
//! `revalidate_task`, `amend` and the criterion `evidence` that success
//! needs, with their Python signatures and their serving. Every argument
//! reaches the use case as the JSON value passed, and each method answers
//! `None`. The decision names what the call did.
use serde_json::Value;

use super::{Parameter, Served, SwarmBoardHandles, done, required, take};
use crate::application::swarm::dto::{
    AmendRunContractRequest, CompleteRunRequest, EvidenceTransition, RecordEvidenceRequest,
    RevalidateTaskRequest,
};
use crate::domain::swarm::BoardError;

/// `complete(revision)`.
pub(super) const COMPLETE: [Parameter; 1] = [required("revision")];
/// `revalidate_task(task_id, revision, evidence)`.
pub(super) const REVALIDATE_TASK: [Parameter; 3] = [
    required("task_id"),
    required("revision"),
    required("evidence"),
];
/// `amend(goal, constraints, criteria, reason)`.
pub(super) const AMEND: [Parameter; 4] = [
    required("goal"),
    required("constraints"),
    required("criteria"),
    required("reason"),
];
/// `evidence(criterion, artifact, revision, kind, passed)`.
pub(super) const EVIDENCE: [Parameter; 5] = [
    required("criterion"),
    required("artifact"),
    required("revision"),
    required("kind"),
    required("passed"),
];

pub(super) fn complete(
    handles: &SwarmBoardHandles,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [revision] = take(arguments)?;
    handles.complete_run.execute(CompleteRunRequest {
        actor: actor.to_owned(),
        revision,
    })?;
    Ok(done("completed"))
}

pub(super) fn revalidate_task(
    handles: &SwarmBoardHandles,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [task_id, revision, evidence] = take(arguments)?;
    handles.revalidate_task.execute(RevalidateTaskRequest {
        actor: actor.to_owned(),
        task_id,
        revision,
        evidence,
    })?;
    Ok(done("revalidated"))
}

pub(super) fn amend(
    handles: &SwarmBoardHandles,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [goal, constraints, criteria, reason] = take(arguments)?;
    handles
        .amend_run_contract
        .execute(AmendRunContractRequest {
            actor: actor.to_owned(),
            goal,
            constraints,
            criteria,
            reason,
        })?;
    Ok(done("amended"))
}

pub(super) fn evidence(
    handles: &SwarmBoardHandles,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [criterion, artifact, revision, kind, passed] = take(arguments)?;
    let transition = handles.record_evidence.execute(RecordEvidenceRequest {
        actor: actor.to_owned(),
        criterion,
        artifact,
        revision,
        kind,
        passed,
    })?;
    Ok(done(match transition {
        EvidenceTransition::Recorded => "recorded",
        EvidenceTransition::Unchanged => "unchanged",
    }))
}

#[cfg(test)]
#[path = "swarm_board_dispatch_completion_tests.rs"]
mod tests;
