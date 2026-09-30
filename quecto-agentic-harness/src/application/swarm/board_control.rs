//! The control reads every run-control use case (#2273) shares: the port
//! of `Coordination._receipt` and `_resume_blockers`, and of the
//! repository's `control_receipt`, `pause_started` and `lost_coordinator`.
//! Capability-internal helpers, not a use case and not a port.
//!
//! A control record only a file edited outside the board can hold (a pause
//! record without a numeric `started`, a boolean included; a budget
//! payload that is not an object or a token limit that is not a count) is
//! refused naming the record, where Python raises a `TypeError` or a
//! `KeyError`, or computes with the value as it is: the
//! `outside_edited_control_records` divergence. Usage totals that are not
//! counts (a REAL or a negative sum) are refused so only where the budget
//! decides under a token limit that is not null (a paused run's resume
//! check, and a running run's recorded request and admission read, where
//! Python compares them with the limit); everywhere else (the receipt's
//! budget, the usage report) they pass through as Python passes them.
//!
//! The receipt and the budget decision read the usage standing (#2340),
//! the budget and the two totals, where Python reads the whole usage
//! report: a ledger row only an edit makes unreadable (a payload that is
//! not JSON among the ten latest, an actor that is not UTF-8) refuses
//! Python's receipt and not this one, part of the same divergence.
use serde_json::Value;

use super::dto::{ControlReceipt, UsageStanding};
use super::ports::{BoardEvents, BoardMembers, BoardRuns, BoardUsage, Clock};
use crate::domain::swarm::{
    BoardError, RefusalKind, RunRecord, UsageBudget, UsageDecision, UsageTotals, python_truthy,
    resume_blockers, usage_budget_decision,
};

/// The refusal of a control record the board never writes.
pub(crate) fn edited(record: &str) -> BoardError {
    BoardError::new(
        RefusalKind::Store,
        format!("the board's {record} is not as the board writes it"),
    )
}

/// The run the operation gate authorised, read again in its transaction.
pub(crate) fn current(transaction: &(impl BoardRuns + ?Sized)) -> Result<RunRecord, BoardError> {
    transaction
        .run()?
        .ok_or_else(|| BoardError::new(RefusalKind::RunMissing, "coordination run missing"))
}

fn paused(run: &RunRecord) -> bool {
    matches!(
        run.status.as_ref().map(|status| status.as_str()),
        Some("paused")
    )
}

/// `Coordination._receipt(tx)`: the control generation, the usage report
/// and the run as `control_receipt` reads them, then the resume blockers.
/// Of the report it reads only the budget and the two totals the receipt
/// carries (#2340), once, and the blockers decide on the same.
///
/// # Errors
/// An edited budget or pause record, or the store's refusal.
pub(crate) fn receipt(
    transaction: &(impl BoardRuns + BoardMembers + BoardEvents + BoardUsage + ?Sized),
    clock: &dyn Clock,
) -> Result<ControlReceipt, BoardError> {
    let generation = transaction.control_generation()?;
    let standing = transaction.usage_standing()?;
    assembled(transaction, clock, generation, standing)
}

/// [`receipt`] on a `standing` the caller read in this transaction and
/// has not changed since but for the budget it read again (#2340): a
/// recorded request's budget decision has just read the totals, which
/// nothing it writes changes.
///
/// # Errors
/// As [`receipt`].
pub(crate) fn receipt_from(
    transaction: &(impl BoardRuns + BoardMembers + BoardEvents + BoardUsage + ?Sized),
    clock: &dyn Clock,
    standing: UsageStanding,
) -> Result<ControlReceipt, BoardError> {
    let generation = transaction.control_generation()?;
    assembled(transaction, clock, generation, standing)
}

/// The receipt of `generation` and `standing`, in Python's order: the run,
/// the budget merged with the totals, then the resume blockers.
fn assembled(
    transaction: &(impl BoardRuns + BoardMembers + BoardUsage + ?Sized),
    clock: &dyn Clock,
    generation: i64,
    standing: UsageStanding,
) -> Result<ControlReceipt, BoardError> {
    let run = current(transaction)?;
    let Value::Object(stored) = &standing.budget else {
        return Err(edited("usage budget"));
    };
    let mut budget = stored.clone();
    budget.insert(
        "observed_tokens".to_owned(),
        standing.observed_tokens.clone(),
    );
    budget.insert(
        "unknown_usage_requests".to_owned(),
        standing.unknown_usage_requests.clone(),
    );
    let resume_blockers = blockers_from(transaction, clock, &run, &standing)?;
    Ok(ControlReceipt {
        status: run.status.as_ref().map(|status| status.as_str().to_owned()),
        outcome: run.outcome,
        reason: run.outcome_reason,
        generation,
        budget,
        resume_blockers,
    })
}

/// `Coordination._resume_blockers(tx)`: why a resume would pause again at
/// once; empty unless the run is paused. The deadline a resume would set
/// adds the paused interval back.
///
/// # Errors
/// A paused run without a pause record, an edited record, or the store's.
pub(crate) fn blockers(
    transaction: &(impl BoardRuns + BoardMembers + BoardUsage + ?Sized),
    clock: &dyn Clock,
) -> Result<Vec<String>, BoardError> {
    let run = current(transaction)?;
    if !paused(&run) {
        return Ok(Vec::new());
    }
    let standing = transaction.usage_standing()?;
    blockers_from(transaction, clock, &run, &standing)
}

/// [`blockers`] of `run` on a `standing` read in this transaction.
fn blockers_from(
    transaction: &(impl BoardRuns + BoardMembers + ?Sized),
    clock: &dyn Clock,
    run: &RunRecord,
    standing: &UsageStanding,
) -> Result<Vec<String>, BoardError> {
    if !paused(run) {
        return Ok(Vec::new());
    }
    let now = clock.now_seconds();
    let (elapsed, _) = paused_for(now, pause_started(transaction)?);
    let deadline = run.deadline + elapsed;
    let decision = budget_decision(standing)?;
    let lost = lost_coordinator(transaction, run)?;
    Ok(resume_blockers(
        deadline,
        now,
        decision.as_str(),
        lost.as_deref(),
    ))
}

/// `Transaction.pause_started()`: when the latest pause began.
///
/// # Errors
/// `paused run has no pause record`, an edited record, or the store's.
pub(crate) fn pause_started(transaction: &(impl BoardRuns + ?Sized)) -> Result<f64, BoardError> {
    let Some(started) = transaction.pause_started()? else {
        return Err(BoardError::new(
            RefusalKind::Internal,
            "paused run has no pause record",
        ));
    };
    started
        .as_f64()
        .filter(|started| started.is_finite())
        .ok_or_else(|| edited("pause record"))
}

/// `max(0, now - started)`: the seconds a run has been paused, and the
/// value Python writes for them, which stays the integer `0` unless the
/// difference is positive (`max` keeps its first argument on a tie).
pub(crate) fn paused_for(now: f64, started: f64) -> (f64, Value) {
    let elapsed = now - started;
    if elapsed > 0.0 {
        (elapsed, Value::from(elapsed))
    } else {
        (0.0, Value::from(0))
    }
}

/// `Transaction.lost_coordinator()`: the coordinator's id when its harness
/// was quarantined after its latest activation (#1924).
fn lost_coordinator(
    transaction: &(impl BoardMembers + ?Sized),
    run: &RunRecord,
) -> Result<Option<String>, BoardError> {
    let Some(coordinator) = run.coordinator.as_deref() else {
        return Ok(None);
    };
    let lost = transaction.lost_members(&[coordinator])?;
    debug_assert!(lost.len() <= 1, "one member asked about: {lost:?}");
    Ok(lost
        .iter()
        .any(|member| member == coordinator)
        .then(|| coordinator.to_owned()))
}

/// `usage_budget_decision(report['budget'], report['totals'])`: no limit
/// allows before anything else is read, as Python's does.
pub(crate) fn budget_decision(standing: &UsageStanding) -> Result<UsageDecision, BoardError> {
    let Value::Object(budget) = &standing.budget else {
        return Err(edited("usage budget"));
    };
    let token_limit = match budget.get("token_limit") {
        Some(Value::Null) => return Ok(UsageDecision::Allow),
        Some(limit) => limit.as_u64().ok_or_else(|| edited("usage budget"))?,
        None => return Err(edited("usage budget")),
    };
    let strict_unknown = budget
        .get("strict_unknown")
        .ok_or_else(|| edited("usage budget"))?;
    let count = |total: &Value| total.as_u64().ok_or_else(|| edited("usage totals record"));
    Ok(usage_budget_decision(
        &UsageBudget {
            token_limit: Some(token_limit),
            strict_unknown: python_truthy(strict_unknown),
            warned: false,
        },
        &UsageTotals {
            observed_tokens: count(&standing.observed_tokens)?,
            unknown_usage_requests: count(&standing.unknown_usage_requests)?,
        },
    ))
}

#[cfg(test)]
#[path = "board_control_tests.rs"]
mod tests;
