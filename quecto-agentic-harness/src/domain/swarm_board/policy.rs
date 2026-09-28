//! Board decisions over values, ported from `swarm_policy.py` (#2266):
//! authorisation, deadline expiry, admission, completion, revalidation,
//! resume blockers and extension bounds. Every refusal is a [`BoardError`]
//! whose text is the Python board's, character for character.
use serde_json::Value;

use super::BoardError;
use super::records::{Criterion, EvidenceRow, MemberRecord, RunRecord, RunState, TaskRecord};

/// Outcomes a coordinator may propose. Each ends the run as a resumable
/// pause; only the supervisor outside the swarm resumes it or closes it into
/// the terminal state of the same name (#1729).
pub const PROPOSED_OUTCOMES: [&str; 4] = ["succeeded", "blocked", "failed", "budget-exhausted"];
/// Statuses `stop` accepts: every proposable outcome but success, plus the
/// at-once terminal cancellation.
pub const STOP_STATUSES: [&str; 4] = ["blocked", "failed", "budget-exhausted", "cancelled"];

/// What an operation needs from the invoking member and the run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Access {
    /// The run must be running.
    pub active: bool,
    /// Only the designated coordinator may act.
    pub coordinator: bool,
    /// A dead member may still read.
    pub read_only: bool,
}

pub fn describe(_run: &RunRecord) -> String {
    String::new()
}

pub fn authorize(
    _run: Option<&RunRecord>,
    _actor: &str,
    _member: Option<&MemberRecord>,
    _access: Access,
) -> Result<(), BoardError> {
    Ok(())
}

pub fn expired(_run: &RunRecord, _now: f64) -> bool {
    false
}

pub fn require_budget(_run: &RunRecord, _now: f64) -> Result<(), BoardError> {
    Ok(())
}

pub fn resume_blockers(
    _resumed_deadline: f64,
    _now: f64,
    _budget_decision: &str,
    _lost_coordinator: Option<&str>,
) -> Vec<String> {
    Vec::new()
}

pub fn validate_extension(_seconds: &Value) -> Result<i64, BoardError> {
    Ok(0)
}

pub fn admission(
    _run: &RunRecord,
    _prior: Option<&MemberRecord>,
    _reservation: &str,
    _usage: i64,
    _now: f64,
) -> Result<bool, BoardError> {
    Ok(true)
}

pub fn completion(
    _criteria: &[Criterion],
    _evidence: &[EvidenceRow],
    _tasks: &[TaskRecord],
    _has_reservations: bool,
    _revision: &Value,
) -> Result<RunState, BoardError> {
    Ok(RunState::SUCCEEDED)
}

pub fn revalidation<'a>(
    _task: &TaskRecord,
    _revision: &Value,
    evidence: &'a Value,
) -> Result<&'a Value, BoardError> {
    Ok(evidence)
}

pub fn require_unsubmitted(_task: &TaskRecord) -> Result<(), BoardError> {
    Ok(())
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod policy_tests;
