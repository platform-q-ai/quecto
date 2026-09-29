//! Board decisions over values, ported from `swarm_policy.py` (#2266):
//! authorisation, deadline expiry, admission, completion, revalidation,
//! resume blockers and extension bounds. Every refusal is a [`BoardError`]
//! whose text is the Python board's, character for character.
use serde_json::Value;

use super::python_value::python_equal;
use super::records::{
    Criterion, EvidenceRow, MemberRecord, MemberState, RunRecord, RunState, TaskRecord,
};
use super::validation::has_content;
use super::{BoardError, RefusalKind};

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

/// The longest deadline extension: seven days.
pub const MAX_EXTENSION_SECONDS: i64 = 7 * 24 * 3600;

/// `run.status` as Python's f-strings write it: the text, or `None` for a
/// NULL status.
fn status_text(run: &RunRecord) -> &str {
    run.status.as_ref().map_or("None", RunState::as_str)
}

/// The status a decision matches on: `None` for a NULL status, which no
/// affirmative arm names.
fn status(run: &RunRecord) -> Option<&str> {
    run.status.as_ref().map(RunState::as_str)
}

/// Status text for errors: a paused run names the outcome it is holding.
pub fn describe(run: &RunRecord) -> String {
    match (status(run), run.outcome.as_deref()) {
        (Some("paused"), Some(outcome)) if !outcome.is_empty() => {
            let reason = run
                .outcome_reason
                .as_deref()
                .filter(|reason| !reason.is_empty());
            format!("paused ({outcome}: {})", reason.unwrap_or("no reason"))
        }
        _ => status_text(run).to_owned(),
    }
}

/// Whether the run's budget is spent (#2303): paused holding the
/// `budget-exhausted` outcome, or ended as `budget-exhausted`.
fn budget_spent(run: &RunRecord) -> bool {
    matches!(
        (status(run), run.outcome.as_deref()),
        (Some("paused"), Some("budget-exhausted")) | (Some("budget-exhausted"), _)
    )
}

/// The kind of a refusal to a run that is not taking work: the budget's
/// when it is spent (or, for `expired`, its deadline has passed), else
/// the run's state.
fn not_running(spent: bool) -> RefusalKind {
    match spent {
        true => RefusalKind::BudgetExhausted,
        false => RefusalKind::NotRunning,
    }
}

/// Whether `actor`, whose row is `member`, may perform an operation needing
/// `access` on `run`. Checks run in Python's order: the run, the coordinator,
/// the member, then activity.
pub fn authorize(
    run: Option<&RunRecord>,
    actor: &str,
    member: Option<&MemberRecord>,
    access: Access,
) -> Result<(), BoardError> {
    let Some(run) = run else {
        return Err(BoardError::new(
            RefusalKind::RunMissing,
            "coordination run missing",
        ));
    };
    let designated = run.coordinator.as_deref() == Some(actor);
    if access.coordinator && !designated {
        return Err(BoardError::new(
            RefusalKind::NotCoordinator,
            "only the designated coordinator may do this",
        ));
    }
    if !member.is_some_and(|member| access.read_only || alive(member)) {
        return Err(BoardError::new(
            RefusalKind::NotMember,
            "invoking member is unknown or death confirmed",
        ));
    }
    match (access.active, status(run)) {
        (false, _) | (true, Some("running")) => Ok(()),
        (true, _) => Err(BoardError::new(
            not_running(budget_spent(run)),
            format!("run is {}; no new work permitted", describe(run)),
        )),
    }
}

/// The member-status allowlist: a member whose death is not confirmed,
/// admitted (`reserved`) or launched (`live`). Python refuses only
/// `'dead'`; any other status (NULL, or one the board never writes) is not
/// alive here, the owner-decided divergence
/// `unknown_member_status_is_not_alive` (#2295). Every board decision on a
/// member's liveness asks this one predicate.
pub fn status_is_alive(status: Option<&str>) -> bool {
    matches!(status, Some("live" | "reserved"))
}

fn alive(member: &MemberRecord) -> bool {
    status_is_alive(member.status.as_ref().map(MemberState::as_str))
}

/// A running run whose deadline has come (a deadline equal to `now` has).
pub fn expired(run: &RunRecord, now: f64) -> bool {
    matches!(status(run), Some("running")) && run.deadline <= now
}

pub fn require_budget(run: &RunRecord, now: f64) -> Result<(), BoardError> {
    if expired(run, now) {
        return Err(BoardError::new(
            RefusalKind::BudgetExhausted,
            "run is paused (budget-exhausted: deadline); no new work permitted",
        ));
    }
    Ok(())
}

/// Why a resume would pause again at once; empty when it may proceed. A
/// coordinator whose harness was lost (#1924) leaves nobody to drive the
/// resumed run, so the run stays paused until that member is relaunched.
pub fn resume_blockers(
    resumed_deadline: f64,
    now: f64,
    budget_decision: &str,
    lost_coordinator: Option<&str>,
) -> Vec<String> {
    let mut blockers = Vec::new();
    if let Some(lost) = lost_coordinator.filter(|lost| !lost.is_empty()) {
        blockers.push(format!(
            "relaunch the lost coordinator '{lost}' into the retained environment before resuming"
        ));
    }
    if resumed_deadline <= now {
        blockers.push("extend the deadline (swarm_control extend) before resuming".to_owned());
    }
    if budget_decision == "pause" {
        blockers.push(
            "raise or disable the token budget (swarm_control usage_budget) before resuming"
                .to_owned(),
        );
    }
    blockers
}

/// A deadline extension: a JSON integer (never a float or a boolean, as
/// Python's `type(seconds) is int`) of 1 through 604800 seconds.
pub fn validate_extension(seconds: &Value) -> Result<i64, BoardError> {
    match seconds.as_i64() {
        Some(seconds @ 1..=MAX_EXTENSION_SECONDS) => Ok(seconds),
        _ => Err(BoardError::new(
            RefusalKind::Invalid,
            "deadline extension must be 1..604800 seconds",
        )),
    }
}

/// Whether to admit a member under `reservation`: `Ok(false)` when `prior`
/// already holds that reservation alive (an idempotent retry), `Ok(true)`
/// for a new admission. `usage` is the count of members whose status is
/// `live` or `reserved`. The reservation is the member's argument as given,
/// compared with the stored one by Python's `==` ([`python_equal`]): a
/// `None` reservation matches a prior `None`, so that retry is idempotent
/// too, while `5` never matches a stored `'5'`. The retry is answered
/// before capacity, and capacity before identity reuse.
pub fn admission(
    run: &RunRecord,
    prior: Option<&MemberRecord>,
    reservation: &Value,
    usage: i64,
    now: f64,
) -> Result<bool, BoardError> {
    let admitting = matches!(status(run), Some("setup" | "running")) && !expired(run, now);
    if !admitting {
        return Err(BoardError::new(
            not_running(budget_spent(run) || expired(run, now)),
            format!("run is {}; no new admission", status_text(run)),
        ));
    }
    let retry = prior.is_some_and(|prior| {
        let held = prior
            .reservation
            .as_deref()
            .map_or(Value::Null, Value::from);
        python_equal(&held, reservation) && alive(prior)
    });
    if retry {
        return Ok(false);
    }
    debug_assert!(
        (1..=25).contains(&run.member_limit),
        "member_limit is 1 through 25, read {}",
        run.member_limit
    );
    if usage >= run.member_limit {
        return Err(BoardError::new(
            RefusalKind::MemberLimit,
            format!(
                "swarm limit {}, current usage {usage}; reuse the existing pool",
                run.member_limit
            ),
        ));
    }
    match prior {
        None => Ok(true),
        Some(_) => Err(BoardError::new(
            RefusalKind::IdentityTaken,
            "member identity already used; choose a stable new identity",
        )),
    }
}

/// Whether the run may succeed at `revision`: every criterion has accepted
/// evidence of its kind at that revision, all work is completed with no file
/// reservation left, and every task's evidence names that revision.
pub fn completion(
    criteria: &[Criterion],
    evidence: &[EvidenceRow],
    tasks: &[TaskRecord],
    has_reservations: bool,
    revision: &Value,
) -> Result<RunState, BoardError> {
    let revision = match revision {
        Value::String(revision) if has_content(revision) => revision.as_str(),
        _ => {
            return Err(BoardError::new(
                RefusalKind::Invalid,
                "completion revision required",
            ));
        }
    };
    let satisfied = |criterion: &Criterion| {
        evidence.iter().any(|row| {
            row.criterion == criterion.id
                && row.revision == revision
                && row.accepted
                && row.kind == criterion.kind
        })
    };
    if criteria.is_empty() || !criteria.iter().all(satisfied) {
        return Err(BoardError::new(
            RefusalKind::CompletionUnmet,
            "completion requires accepted evidence at the current revision for every criterion",
        ));
    }
    let settled = !has_reservations && tasks.iter().all(|task| task.status.as_str() == "completed");
    if !settled {
        return Err(BoardError::new(
            RefusalKind::CompletionUnmet,
            "settle outstanding work and file reservations before success",
        ));
    }
    let current = |task: &TaskRecord| {
        task.evidence.as_array().is_some_and(|entries| {
            !entries.is_empty()
                && entries
                    .iter()
                    .all(|entry| entry.get("revision").and_then(Value::as_str) == Some(revision))
        })
    };
    if !tasks.iter().all(current) {
        return Err(BoardError::new(
            RefusalKind::StaleRevision,
            "task evidence refers to stale revision",
        ));
    }
    Ok(RunState::SUCCEEDED)
}

/// New evidence for a completed task at `revision`: a nonempty list of
/// `{artifact, revision}` objects, each with an artifact and that revision.
/// Returns the evidence unchanged, extra keys included.
pub fn revalidation<'a>(
    task: &TaskRecord,
    revision: &Value,
    evidence: &'a Value,
) -> Result<&'a Value, BoardError> {
    match task.status.as_str() {
        "completed" => {}
        _ => {
            return Err(BoardError::new(
                RefusalKind::WrongState,
                "only completed tasks may be revalidated",
            ));
        }
    }
    let (revision, entries) = match (revision, evidence) {
        (Value::String(text), Value::Array(entries))
            if has_content(text) && !entries.is_empty() =>
        {
            (revision, entries)
        }
        _ => {
            return Err(BoardError::new(
                RefusalKind::Invalid,
                "new artifact and revision evidence required",
            ));
        }
    };
    let matching = |entry: &Value| {
        entry.is_object()
            && entry
                .get("artifact")
                .and_then(Value::as_str)
                .is_some_and(has_content)
            && entry.get("revision") == Some(revision)
    };
    if !entries.iter().all(matching) {
        return Err(BoardError::new(
            RefusalKind::Invalid,
            "new artifact evidence must match the revalidated revision",
        ));
    }
    Ok(evidence)
}

/// Submitted evidence is immutable: only a task in another known status may
/// be revised. An unknown status found in a file is refused by name.
pub fn require_unsubmitted(task: &TaskRecord) -> Result<(), BoardError> {
    match task.status.as_str() {
        "ready" | "claimed" | "blocked" | "completed" => Ok(()),
        "submitted" => Err(BoardError::new(
            RefusalKind::Immutable,
            "submitted evidence is immutable; release and reclaim before revising",
        )),
        unknown => Err(BoardError::new(
            RefusalKind::WrongState,
            format!("task status '{unknown}' is not a known status; no revision permitted"),
        )),
    }
}

#[cfg(test)]
#[path = "policy_tests.rs"]
mod policy_tests;

#[cfg(test)]
#[path = "policy_null_status_tests.rs"]
mod policy_null_status_tests;

#[cfg(test)]
#[path = "policy_membership_tests.rs"]
mod policy_membership_tests;
