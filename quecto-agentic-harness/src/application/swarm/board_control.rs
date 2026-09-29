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
//! counts (a REAL or a negative sum) are refused so only where a paused
//! run's budget, with a token limit that is not null, is checked for a
//! resume; everywhere else (the receipt's budget, the usage report) they
//! pass through as Python passes them.
use serde_json::Value;

use super::dto::{ControlReceipt, UsageReport};
use super::ports::{BoardEvents, BoardMembers, BoardRuns, BoardUsage, Clock};
use crate::domain::swarm::{
    BoardError, RefusalKind, RunRecord, UsageBudget, UsageDecision, UsageTotals, python_truthy,
    resume_blockers, usage_budget_decision,
};

/// The refusal of a control record the board never writes.
fn edited(record: &str) -> BoardError {
    BoardError::new(
        RefusalKind::Store,
        format!("the board's {record} is not as the board writes it"),
    )
}

/// The run the operation gate authorised, read again in its transaction.
fn current(transaction: &(impl BoardRuns + ?Sized)) -> Result<RunRecord, BoardError> {
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
///
/// # Errors
/// An edited budget or pause record, or the store's refusal.
pub(crate) fn receipt(
    transaction: &(impl BoardRuns + BoardMembers + BoardEvents + BoardUsage + ?Sized),
    clock: &dyn Clock,
) -> Result<ControlReceipt, BoardError> {
    let generation = transaction.control_generation()?;
    let report = transaction.usage_report()?;
    let run = current(transaction)?;
    let Value::Object(mut budget) = report.budget else {
        return Err(edited("usage budget"));
    };
    for key in ["observed_tokens", "unknown_usage_requests"] {
        let total = report
            .totals
            .get(key)
            .ok_or_else(|| edited("usage totals record"))?;
        budget.insert(key.to_owned(), total.clone());
    }
    Ok(ControlReceipt {
        status: run.status.as_ref().map(|status| status.as_str().to_owned()),
        outcome: run.outcome.clone(),
        reason: run.outcome_reason.clone(),
        generation,
        budget,
        resume_blockers: blockers(transaction, clock)?,
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
    let now = clock.now_seconds();
    let (elapsed, _) = paused_for(now, pause_started(transaction)?);
    let deadline = run.deadline + elapsed;
    let decision = budget_decision(&transaction.usage_report()?)?;
    let lost = lost_coordinator(transaction, &run)?;
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
fn budget_decision(report: &UsageReport) -> Result<UsageDecision, BoardError> {
    let Value::Object(budget) = &report.budget else {
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
    let count = |key: &str| {
        report
            .totals
            .get(key)
            .and_then(Value::as_u64)
            .ok_or_else(|| edited("usage totals record"))
    };
    Ok(usage_budget_decision(
        &UsageBudget {
            token_limit: Some(token_limit),
            strict_unknown: python_truthy(strict_unknown),
            warned: false,
        },
        &UsageTotals {
            observed_tokens: count("observed_tokens")?,
            unknown_usage_requests: count("unknown_usage_requests")?,
        },
    ))
}

#[cfg(test)]
#[path = "board_control_tests.rs"]
mod tests;
