//! A task's runs, each its own artifact (#2481's artifact model). A run
//! begins only while its task is held, by its holder, one at a time; it
//! folds back once, when it ends. One that will never report its end (its
//! host died, or its holder walked away) is ended as `lost`: by the task's
//! holder (whoever takes an expired claim over included), by anyone once
//! no live claim answers for it, or by the completion that makes its task
//! done. A run going keeps its task held: the task does not leave its
//! holder until the run has ended.
//!
//! Every rule takes all of the task's runs as the board holds them, and
//! refuses runs that contradict each other (more than one going) rather
//! than choose among them.
use super::super::entities::claim::Identity;
use super::super::entities::plan::Plan;
use super::super::entities::run::{MAX_RUN_NUMBER, Run, RunFields, RunItem};
use super::super::entities::task::Task;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::schema_error::SchemaError;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::{PlanState, RunOutcome, TaskStatus};
use super::claims::{held_live_by, stamp};

/// A new run of `task`'s accepted `plan`, begun by `actor`, the holder of
/// a live claim on it, at `now`, numbered after every run in `runs`.
///
/// # Errors
/// Not the holder of a live claim; a run of another task, or runs that
/// contradict each other; a run still going; a plan of another task, or
/// one that is not the task's accepted plan; no number left.
pub fn begin_run(
    task: &Task,
    runs: &[Run],
    plan: &Plan,
    actor: &Identity,
    now: &Timestamp,
) -> Result<Run, BoardRuleError> {
    // A live claim held by the actor: the task is held, by them.
    held_live_by(task, actor, now)?;
    let fields = task.fields();
    if let Some(going) = open_of(task, runs)? {
        return Err(BoardRuleError::RunOpen(going.fields().number));
    }
    let planned = plan.fields();
    match planned.task == fields.id {
        true => {}
        false => return Err(BoardRuleError::NotThisTasks),
    }
    let accepted =
        planned.state == PlanState::Accepted && fields.accepted_plan == Some(planned.version);
    match accepted {
        true => {}
        false => return Err(BoardRuleError::PlanNotAccepted(planned.version)),
    }
    let last = runs
        .iter()
        .map(|run| run.fields().number)
        .max()
        .unwrap_or(0);
    match last < MAX_RUN_NUMBER {
        true => {}
        false => {
            return Err(BoardRuleError::Invalid(SchemaError::new(
                "number",
                format!("the task has reached {MAX_RUN_NUMBER} runs"),
            )));
        }
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

/// The open `run` of `task` ended as [`RunOutcome::Lost`] by `actor`: the
/// holder of a live claim on the task (whoever finds a claim expired takes
/// the task over first, and so becomes its holder), or anyone when nobody
/// claims the task, so a run is never stranded. It ends at the task's
/// stamp for `now` (never before the task's last change).
///
/// # Errors
/// A claim `actor` does not hold live; a run of another task, or one that
/// has ended; a stamp the board cannot spell.
pub fn end_lost(
    task: &Task,
    run: &Run,
    actor: &Identity,
    now: &Timestamp,
) -> Result<Run, BoardRuleError> {
    // A claimed task's run is its holder's (one whose claim expired takes
    // the task over first); only a task nobody claims leaves it to anyone.
    if task.fields().claim.is_some() {
        held_live_by(task, actor, now)?;
    }
    of_task(task, std::slice::from_ref(run))?;
    lost_at(run, &stamp(task, now)?)
}

/// A task may leave its holder (for ready, done or archived) only once
/// its runs have ended; it may move among the held statuses while one is
/// going.
///
/// # Errors
/// A run of another task, or runs that contradict each other; a run
/// still going while `to` is not a held status.
pub fn may_move_with(task: &Task, runs: &[Run], to: TaskStatus) -> Result<(), BoardRuleError> {
    let open = open_of(task, runs)?;
    match (open, to.may_hold_claim()) {
        (None, _) | (Some(_), true) => Ok(()),
        (Some(run), false) => Err(BoardRuleError::RunOpen(run.fields().number)),
    }
}

/// The task's run going, if one is: every run in `runs` is the task's, and
/// at most one is going.
///
/// # Errors
/// A run of another task; more than one going.
pub(super) fn open_of<'a>(task: &Task, runs: &'a [Run]) -> Result<Option<&'a Run>, BoardRuleError> {
    of_task(task, runs)?;
    let going: Vec<&Run> = runs.iter().filter(|run| run.is_open()).collect();
    match going.as_slice() {
        [] => Ok(None),
        [run] => Ok(Some(run)),
        [..] => {
            let numbers: Vec<String> = going
                .iter()
                .map(|run| run.fields().number.to_string())
                .collect();
            Err(BoardRuleError::RunsInconsistent(format!(
                "runs {} are all going; a task has one at a time",
                numbers.join(", ")
            )))
        }
    }
}

/// `run` ended as lost at `at`, whoever ends it: [`end_lost`], or a
/// completion that makes its task done (at the completion's stamp).
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
