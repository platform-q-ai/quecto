//! A task's runs, each its own artifact (#2481's artifact model). A run is
//! folded back once, when it ends; one that will never report its end (its
//! host died, or its holder walked away) is ended as `lost` by the task's
//! holder, which includes whoever takes an expired claim over; then the
//! task can be handed back or finished. A run still going keeps its task
//! held: the task does not leave its holder until the run has ended.
use super::super::entities::claim::Identity;
use super::super::entities::plan::Plan;
use super::super::entities::run::{Run, RunItem};
use super::super::entities::task::Task;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::{RunOutcome, TaskStatus};
use super::claims::held_live_by;

/// `run` folded back at `now`: its `outcome`, its `cost` and where each of
/// `plan`'s items stood. A run folds back once.
///
/// # Errors
/// A run that has ended; a run, plan or item of another task or version;
/// or a result its schema refuses.
pub fn fold_back(
    run: &Run,
    plan: &Plan,
    outcome: RunOutcome,
    cost: Option<u64>,
    items: Vec<RunItem>,
    now: &Timestamp,
) -> Result<Run, BoardRuleError> {
    ended(run, plan, outcome, cost, items, now)
}

/// The open run of `task` ended as [`RunOutcome::Lost`] at `now`, by the
/// holder of a live claim on the task.
///
/// # Errors
/// Not the holder of a live claim; a run of another task, or one that has
/// ended.
pub fn end_lost(
    task: &Task,
    run: &Run,
    plan: &Plan,
    actor: &Identity,
    now: &Timestamp,
) -> Result<Run, BoardRuleError> {
    held_live_by(task, actor, now)?;
    if run.fields().task != task.fields().id {
        return Err(BoardRuleError::NotThisTasks);
    }
    ended(run, plan, RunOutcome::Lost, None, vec![], now)
}

/// A task may leave its holder (for ready, done or archived) only once
/// its runs have ended; it may move among the held statuses while one is
/// going.
///
/// # Errors
/// `open` is a run still going and `to` is not a held status.
pub fn may_move_with(open: Option<&Run>, to: TaskStatus) -> Result<(), BoardRuleError> {
    match open.filter(|run| run.is_open()) {
        None => Ok(()),
        Some(_) if to.may_hold_claim() => Ok(()),
        Some(run) => Err(BoardRuleError::RunOpen(run.fields().number)),
    }
}

fn ended(
    run: &Run,
    plan: &Plan,
    outcome: RunOutcome,
    cost: Option<u64>,
    items: Vec<RunItem>,
    now: &Timestamp,
) -> Result<Run, BoardRuleError> {
    if !run.is_open() {
        return Err(BoardRuleError::RunEnded(run.fields().number));
    }
    run.ran(plan).map_err(|_| BoardRuleError::NotThisTasks)?;
    let mut next = run.fields().clone();
    // A run can end within the second it started, never before.
    next.ended = Some(now.clone().max(next.started.clone()));
    next.outcome = Some(outcome);
    next.cost = cost;
    next.items = items;
    let next = Run::new(next).map_err(BoardRuleError::Invalid)?;
    next.ran(plan).map_err(|_| BoardRuleError::NotThisTasks)?;
    Ok(next)
}

#[cfg(test)]
#[path = "runs_tests.rs"]
mod tests;
