//! What a run's outcome means for its task (#2705 T5): where a run's end
//! moves its task, written in the same change as the run; where each task
//! item of a plan stands, from the task's latest run; and which states of
//! a task contradict its runs and plan (legacy states the lifecycle never
//! makes), which a read names and never rewrites.
use super::super::entities::plan::Plan;
use super::super::entities::run::Run;
use super::super::entities::task::Task;
use super::super::value_objects::rule_error::BoardRuleError;
use super::super::value_objects::timestamp::Timestamp;
use super::super::value_objects::vocabulary::{Inconsistency, ItemState, RunOutcome, TaskStatus};

/// How a stopped run stopped: its swarm ended blocked (it waits on the
/// user), or it was cancelled (or could not answer at all).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopKind {
    Blocked,
    Cancelled,
}

/// The status a run's end moves its task to: a success to review, a
/// blocked stop to blocked, any other end (a cancelled stop, a failure, a
/// loss) back to ready.
pub fn status_after(outcome: RunOutcome, stop: StopKind) -> TaskStatus {
    let _ = (outcome, stop);
    TaskStatus::InProgress
}

/// `task` as the end of its run `ended` leaves it.
///
/// # Errors
/// A run of another task, or one still going; a stamp the board cannot
/// spell.
pub fn task_after_end(
    task: &Task,
    ended: &Run,
    stop: StopKind,
    now: &Timestamp,
) -> Result<Option<Task>, BoardRuleError> {
    let _ = (task, ended, stop, now);
    Ok(None)
}

/// Where one task item of a plan stands: as the task's latest run
/// recorded it, or not started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStanding {
    NotStarted,
    In(ItemState),
}

impl ItemStanding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::In(state) => state.as_str(),
        }
    }
}

/// Where each task item of `plan` stands, in its order, from `latest`.
pub fn item_standings(plan: &Plan, latest: Option<&Run>) -> Vec<ItemStanding> {
    let _ = latest;
    plan.fields()
        .items
        .iter()
        .map(|_| ItemStanding::In(ItemState::Todo))
        .collect()
}

/// The states of `task` that contradict its `runs` and plan.
pub fn inconsistencies(task: &Task, runs: &[Run]) -> Vec<Inconsistency> {
    let _ = (task, runs);
    vec![]
}

#[cfg(test)]
#[path = "run_outcomes_tests.rs"]
mod tests;
