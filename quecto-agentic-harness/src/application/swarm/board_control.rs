//! The control reads every run-control use case (#2273) shares: the port
//! of `Coordination._receipt` and `_resume_blockers`, and of the
//! repository's `control_receipt`, `pause_started` and `lost_coordinator`.
//! Capability-internal helpers, not a use case and not a port.
//!
//! A control record only a file edited outside the board can hold (a pause
//! record without a numeric `started`, a boolean included; a budget
//! payload that is not an object) is refused naming the record, where
//! Python raises a `TypeError` or a `KeyError`, or computes with the value
//! as it is: the `outside_edited_control_records` divergence. Usage totals
//! that are not counts (a REAL or a negative sum) pass through as Python
//! passes them: no token budget decides on them (owner decision,
//! 2026-10-14), so a stored budget, `warned` included, is only reported.
//!
//! The receipt reads the usage standing (#2340), the budget and the two
//! totals, where Python reads the whole usage report: a ledger row only an
//! edit makes unreadable (a payload that is not JSON among the ten latest,
//! an actor that is not UTF-8) refuses Python's receipt and not this one,
//! part of the same divergence.
use serde_json::{Map, Value};

use super::dto::{ControlReceipt, UsageStanding};
use super::ports::{BoardEvents, BoardMembers, BoardRuns, BoardUsage, Clock};
use crate::domain::swarm::{BoardError, RefusalKind, RunRecord, resume_blockers};

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
/// carries (#2340), once.
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
/// has not changed since (#2340): a recorded request reads the totals
/// once, after its write.
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

/// The settings of no budget, as a board without a budget row reads them.
fn no_budget() -> Map<String, Value> {
    Map::from_iter([
        ("token_limit".to_owned(), Value::Null),
        ("strict_unknown".to_owned(), Value::Bool(false)),
        ("warned".to_owned(), Value::Bool(false)),
    ])
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
    // A budget is reported, never applied (owner decision, 2026-10-14): one
    // no board writes reads as no budget (review round 1, #2569).
    let mut budget = match &standing.budget {
        Value::Object(stored) => stored.clone(),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            no_budget()
        }
    };
    budget.insert(
        "observed_tokens".to_owned(),
        standing.observed_tokens.clone(),
    );
    budget.insert(
        "unknown_usage_requests".to_owned(),
        standing.unknown_usage_requests.clone(),
    );
    let resume_blockers = blockers_from(transaction, clock, &run)?;
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
    transaction: &(impl BoardRuns + BoardMembers + ?Sized),
    clock: &dyn Clock,
) -> Result<Vec<String>, BoardError> {
    let run = current(transaction)?;
    blockers_from(transaction, clock, &run)
}

/// [`blockers`] of `run`, read in this transaction: its deadline and its
/// coordinator, never its usage (no token budget blocks a resume).
fn blockers_from(
    transaction: &(impl BoardRuns + BoardMembers + ?Sized),
    clock: &dyn Clock,
    run: &RunRecord,
) -> Result<Vec<String>, BoardError> {
    if !paused(run) {
        return Ok(Vec::new());
    }
    let now = clock.now_seconds();
    let (elapsed, _) = paused_for(now, pause_started(transaction)?);
    let deadline = run.deadline + elapsed;
    let lost = lost_coordinator(transaction, run)?;
    Ok(resume_blockers(deadline, now, lost.as_deref()))
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

#[cfg(test)]
#[path = "board_control_tests.rs"]
mod tests;
