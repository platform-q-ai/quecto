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
use super::super::value_objects::vocabulary::TaskStatus::{self, *};
use super::super::value_objects::vocabulary::{Inconsistency, ItemState, Resolution, RunOutcome};
use super::claims::{settle, stamp};
use super::runs::of_task;

/// How a stopped run stopped: its swarm ended blocked (it waits on the
/// user), or it was cancelled (or could not answer at all).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopKind {
    Blocked,
    Cancelled,
}

/// The status a run's end moves its task to: a success to review, a
/// blocked stop to blocked, any other end (a cancelled stop, a failure, a
/// loss) back to ready. `stop` counts only for a stop.
pub fn status_after(outcome: RunOutcome, stop: StopKind) -> TaskStatus {
    match (outcome, stop) {
        (RunOutcome::Succeeded, _) => Review,
        (RunOutcome::Stopped, StopKind::Blocked) => Blocked,
        (RunOutcome::Stopped, StopKind::Cancelled) | (RunOutcome::Failed | RunOutcome::Lost, _) => {
            Ready
        }
    }
}

/// `task` as the end of its run `ended` leaves it, at the task's stamp for
/// `now` (the stamp the run ended at): moved to [`status_after`]; back to
/// ready with its claim released, or to review or blocked with its
/// holder's claim kept. Only a held task moves (claimed, in progress, in
/// review or blocked: the statuses a run going keeps a task in); one
/// nobody holds (a legacy state the lifecycle never makes) and one already
/// where the end moves it are left as they are (`None`), and the run is
/// written alone.
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
    of_task(task, std::slice::from_ref(ended))?;
    let fields = task.fields();
    let outcome = ended
        .fields()
        .outcome
        .ok_or(BoardRuleError::RunOpen(ended.fields().number))?;
    match fields.status {
        Claimed | InProgress | Review | Blocked => {}
        Draft | Ready | Done | Archived => return Ok(None),
    }
    let to = status_after(outcome, stop);
    let claim = match to {
        Ready => None,
        Review | Blocked => fields.claim.clone(),
        Draft | Claimed | InProgress | Done | Archived => {
            unreachable!("a run's end moves its task to ready, review or blocked, not {to:?}")
        }
    };
    if (to, &claim) == (fields.status, &fields.claim) {
        return Ok(None);
    }
    assert!(
        fields.resolution.is_none() && fields.reason.is_none(),
        "a held task is unresolved and keeps no reason"
    );
    let at = stamp(task, now)?;
    let mut next = fields.clone();
    next.status = to;
    next.claim = claim;
    settle(next, &at).map(Some)
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

/// Where each task item of `plan` stands, in its order, from `latest`,
/// the task's latest run: not started while there is none, or it ran
/// another version of the plan; otherwise as it recorded the item, and
/// todo while it records none (a run going records its items when it
/// ends).
pub fn item_standings(plan: &Plan, latest: Option<&Run>) -> Vec<ItemStanding> {
    let planned = plan.fields();
    let ran = latest.filter(|run| run.fields().plan_version == planned.version);
    planned
        .items
        .iter()
        .map(|item| match ran {
            None => ItemStanding::NotStarted,
            Some(run) => {
                let recorded = run.fields().items.iter().find(|ran| ran.id == item.id);
                ItemStanding::In(recorded.map_or(ItemState::Todo, |ran| ran.state))
            }
        })
        .collect()
}

/// The states of `task` that contradict its `runs` (every run the board
/// holds of it) and its plan, as the lifecycle never leaves them:
/// - `done_without_plan`: completed with no plan accepted and no run (a
///   swarm spawned by hand, the land-layout-2652 case), though done
///   without a run says so in its resolution;
/// - `in_progress_without_run`: in progress with no run going;
/// - `run_going_outside_progress`: a run going while the task is in any
///   other status.
pub fn inconsistencies(task: &Task, runs: &[Run]) -> Vec<Inconsistency> {
    let fields = task.fields();
    let going = runs.iter().any(Run::is_open);
    let done_without_plan = fields.status == Done
        && fields.resolution == Some(Resolution::Completed)
        && fields.accepted_plan.is_none()
        && runs.is_empty();
    let (in_progress, outside) = match fields.status {
        InProgress => (!going, false),
        Draft | Ready | Claimed | Review | Blocked | Done | Archived => (false, going),
    };
    [
        (done_without_plan, Inconsistency::DoneWithoutPlan),
        (in_progress, Inconsistency::InProgressWithoutRun),
        (outside, Inconsistency::RunGoingOutsideProgress),
    ]
    .into_iter()
    .filter(|(found, _)| *found)
    .map(|(_, inconsistency)| inconsistency)
    .collect()
}

#[cfg(test)]
#[path = "run_outcomes_tests.rs"]
mod tests;
