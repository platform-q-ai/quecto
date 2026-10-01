//! The completion methods of the board dispatch (#2273): `complete`,
//! `revalidate_task`, `amend` and the criterion `evidence` that success
//! needs, with their Python signatures and their serving. Every argument
//! reaches the use case as the JSON value passed. Where Python answered
//! `None`, each answers what it changed (#2394): `complete` the run's
//! `{status, outcome, reason}` (as `stop`'s receipt opens),
//! `revalidate_task` the task's dict, `amend` the contract and `evidence`
//! the evidence as recorded. The decision names what the call did;
//! `revalidate_task`'s record names the task its row holds (#2303), and the
//! others act on no task, message or cursor.
use serde_json::Value;

use super::tasks::acted_on;
use super::{Parameter, Served, done, object, required, take};
use crate::application::swarm::dto::{
    AmendRunContractRequest, CompleteRunRequest, EvidenceTransition, NewEvidence,
    RecordEvidenceRequest, RevalidateTaskRequest, StoredContract,
};
use crate::application::swarm::use_cases::{
    AmendRunContract, CompleteRun, RecordEvidence, RevalidateTask,
};
use crate::domain::swarm::{BoardError, RunRecord};

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
    complete_run: &CompleteRun,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [revision] = take(arguments)?;
    let ended = complete_run.execute(CompleteRunRequest {
        actor: actor.to_owned(),
        revision,
    })?;
    Ok(Served {
        value: ended_run(ended),
        controls_run: true,
        ..done("completed")
    })
}

/// Records the task by the id its row holds (#2303), as the task methods'
/// records do: `"1"` acts on task 1.
pub(super) fn revalidate_task(
    revalidate_task: &RevalidateTask,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [task_id, revision, evidence] = take(arguments)?;
    let task = revalidate_task.execute(RevalidateTaskRequest {
        actor: actor.to_owned(),
        task_id,
        revision,
        evidence,
    })?;
    Ok(Served {
        task_id: acted_on(task.get("id")),
        value: task.into_value(),
        ..done("revalidated")
    })
}

pub(super) fn amend(
    amend_run_contract: &AmendRunContract,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [goal, constraints, criteria, reason] = take(arguments)?;
    let amended = amend_run_contract.execute(AmendRunContractRequest {
        actor: actor.to_owned(),
        goal,
        constraints,
        criteria,
        reason,
    })?;
    Ok(Served {
        value: contract(amended),
        ..done("amended")
    })
}

/// `{status, outcome, reason}`: the run as it now stands, in the order the
/// control receipt opens with.
fn ended_run(run: RunRecord) -> Value {
    let text = |value: Option<String>| value.map_or(Value::Null, Value::String);
    object([
        (
            "status",
            text(run.status.map(|status| status.as_str().to_owned())),
        ),
        ("outcome", text(run.outcome)),
        ("reason", text(run.outcome_reason)),
    ])
}

/// `{goal, constraints, criteria}`: the run's contract as it now stands.
fn contract(contract: StoredContract) -> Value {
    object([
        ("goal", contract.goal),
        ("constraints", contract.constraints),
        ("criteria", contract.criteria),
    ])
}

/// The evidence as recorded, in the `evidence` table's column order, its
/// criterion as the row stores it.
fn recorded(criterion: Value, evidence: NewEvidence) -> Value {
    object([
        ("criterion", criterion),
        ("artifact", Value::from(evidence.artifact)),
        ("revision", Value::from(evidence.revision)),
        ("kind", Value::from(evidence.kind)),
        ("actor", Value::from(evidence.actor)),
        ("accepted", Value::Bool(evidence.accepted)),
    ])
}

pub(super) fn evidence(
    record_evidence: &RecordEvidence,
    actor: &str,
    arguments: Vec<Value>,
) -> Result<Served, BoardError> {
    let [criterion, artifact, revision, kind, passed] = take(arguments)?;
    let recorded_evidence = record_evidence.execute(RecordEvidenceRequest {
        actor: actor.to_owned(),
        criterion,
        artifact,
        revision,
        kind,
        passed,
    })?;
    Ok(Served {
        value: recorded(recorded_evidence.criterion, recorded_evidence.evidence),
        ..done(match recorded_evidence.transition {
            EvidenceTransition::Recorded => "recorded",
            EvidenceTransition::Unchanged => "unchanged",
        })
    })
}

#[cfg(test)]
#[path = "swarm_board_dispatch_completion_tests.rs"]
mod tests;
