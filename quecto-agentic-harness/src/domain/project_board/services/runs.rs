//! A task's runs, each its own artifact (#2481's artifact model). A run
//! opens only while its task is held, one at a time; it folds back once,
//! when it ends; one that will never report its end (its host died, or its
//! holder walked away) is ended as `lost` by the task's holder, which
//! includes whoever takes an expired claim over, or by the completion that
//! makes its task done. A run still going keeps its task held: the task
//! does not leave its holder until the run has ended.
use super::super::entities::claim::Identity;
use super::super::entities::plan::Plan;
use super::super::entities::run::{MAX_RUN_NUMBER, Run, RunFields, RunItem};
use super::super::entities::task::Task;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::schema_error::SchemaError;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::{PlanState, RunOutcome, TaskStatus};
use super::claims::held_live_by;

/// A new run of `task`'s accepted `plan`, opened at `now`, numbered after
/// every run in `runs`.
///
/// # Errors
/// A task nobody holds; a run of another task in `runs`; a run still
/// going; a plan that is not the task's accepted one; no number left.
pub fn begin_run(
    task: &Task,
    runs: &[Run],
    plan: &Plan,
    now: &Timestamp,
) -> Result<Run, BoardRuleError> {
    let fields = task.fields();
    match fields.claim.is_some() && fields.status.may_hold_claim() {
        true => {}
        false => return Err(BoardRuleError::NotHeld),
    }
    of_task(task, runs)?;
    if let Some(going) = runs.iter().find(|run| run.is_open()) {
        return Err(BoardRuleError::RunOpen(going.fields().number));
    }
    let planned = plan.fields();
    let accepted = planned.state == PlanState::Accepted
        && planned.task == fields.id
        && fields.accepted_plan == Some(planned.version);
    match accepted {
        true => {}
        false => return Err(BoardRuleError::NotThisTasks),
    }
    let last = runs
        .iter()
        .map(|run| run.fields().number)
        .max()
        .unwrap_or(0);
    if last >= MAX_RUN_NUMBER {
        return Err(BoardRuleError::Invalid(SchemaError::new(
            "number",
            format!("the task has reached {MAX_RUN_NUMBER} runs"),
        )));
    }
    Run::new(RunFields {
        task: fields.id.clone(),
        number: last + 1,
        plan_version: planned.version,
        started: now.clone(),
        ended: None,
        outcome: None,
        cost: None,
        items: vec![],
    })
    .map_err(BoardRuleError::Invalid)
}

/// `run` folded back at `now`: its `outcome`, its `cost` and where each of
/// `plan`'s items stood. A run folds back once.
///
/// # Errors
/// A run that has ended; a plan never accepted, or of another task or
/// version, or an item it does not have (named by the error's field); or
/// a result its schema refuses.
pub fn fold_back(
    run: &Run,
    plan: &Plan,
    outcome: RunOutcome,
    cost: Option<u64>,
    items: Vec<RunItem>,
    now: &Timestamp,
) -> Result<Run, BoardRuleError> {
    run.ran(plan).map_err(BoardRuleError::Invalid)?;
    let folded = ended(run, outcome, cost, items, now)?;
    folded.ran(plan).map_err(BoardRuleError::Invalid)?;
    Ok(folded)
}

/// The open `run` of `task` ended as [`RunOutcome::Lost`] at `now`, by the
/// holder of a live claim on the task.
///
/// # Errors
/// Not the holder of a live claim; a run of another task, or one that has
/// ended.
pub fn end_lost(
    task: &Task,
    run: &Run,
    actor: &Identity,
    now: &Timestamp,
) -> Result<Run, BoardRuleError> {
    held_live_by(task, actor, now)?;
    of_task(task, std::slice::from_ref(run))?;
    lost_at(run, now)
}

/// A task may leave its holder (for ready, done or archived) only once
/// its run has ended; it may move among the held statuses while one is
/// going.
///
/// # Errors
/// `open` is a run of another task, or one still going while `to` is not
/// a held status.
pub fn may_move_with(
    task: &Task,
    open: Option<&Run>,
    to: TaskStatus,
) -> Result<(), BoardRuleError> {
    let Some(run) = open else {
        return Ok(());
    };
    of_task(task, std::slice::from_ref(run))?;
    match (run.is_open(), to.may_hold_claim()) {
        (false, _) | (true, true) => Ok(()),
        (true, false) => Err(BoardRuleError::RunOpen(run.fields().number)),
    }
}

/// `run` ended as lost at `at`, whoever ends it: the holder's
/// [`end_lost`], or a completion that makes its task done.
pub(super) fn lost_at(run: &Run, at: &Timestamp) -> Result<Run, BoardRuleError> {
    ended(run, RunOutcome::Lost, None, vec![], at)
}

/// Every run in `runs` is `task`'s.
fn of_task(task: &Task, runs: &[Run]) -> Result<(), BoardRuleError> {
    match runs.iter().all(|run| run.fields().task == task.fields().id) {
        true => Ok(()),
        false => Err(BoardRuleError::NotThisTasks),
    }
}

fn ended(
    run: &Run,
    outcome: RunOutcome,
    cost: Option<u64>,
    items: Vec<RunItem>,
    now: &Timestamp,
) -> Result<Run, BoardRuleError> {
    match run.is_open() {
        true => {}
        false => return Err(BoardRuleError::RunEnded(run.fields().number)),
    }
    let mut next = run.fields().clone();
    // A run can end within the second it started, never before.
    next.ended = Some(now.clone().max(next.started.clone()));
    next.outcome = Some(outcome);
    next.cost = cost;
    next.items = items;
    Run::new(next).map_err(BoardRuleError::Invalid)
}

#[cfg(test)]
#[path = "runs_tests.rs"]
mod tests;
