//! What a listing says of each task it lists: the `summary` beside each
//! task of a `list`, `list_ready` and `list_takeable` answer. It is read
//! from the same read of the board as the task's row ([`ShownTask`], as
//! `show` reads it), so the two always agree.
//!
//! - **plan:** the newest version of the task's plan that is a draft or
//!   accepted (superseded and discarded versions are skipped), so a draft
//!   newer than the accepted plan wins; `None` when there is none.
//! - **run:** the task's latest readable run ([`ShownTask::latest_run`]):
//!   its number, plan version, outcome (`None` while it runs), when it
//!   started and ended, its deadline, and how many of its task items stand
//!   in each [`ItemState`]. A run records its task items when it ends, so
//!   a running run counts none.
//! - **deadline:** `started` plus the deadline of the run's plan's team,
//!   by the rule a start briefs a swarm with
//!   ([`deadline_seconds`]); `None` while the plan the run ran is not
//!   readable on the board, or past the year 9999.
//! - **may_be_incomplete:** as `show` judges it
//!   ([`ShownTask::may_be_incomplete`]).
use super::dto::{BoardListing, ShownTask};
use super::ports::{BoardHead, QuarantinedFile};
use super::swarm_brief::deadline_seconds;
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::entities::run::{Run, RunItem};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::{ItemState, PlanState, RunOutcome};

/// What a listing says of one task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSummary {
    pub plan: Option<PlanSummary>,
    pub run: Option<RunSummary>,
    pub may_be_incomplete: bool,
}

/// The task's current plan version: a draft or accepted one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanSummary {
    pub version: u32,
    /// [`PlanState::Draft`] or [`PlanState::Accepted`].
    pub state: PlanState,
}

/// The task's latest readable run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummary {
    pub number: u32,
    pub plan_version: u32,
    /// `None` while it runs.
    pub outcome: Option<RunOutcome>,
    pub started: Timestamp,
    pub ended: Option<Timestamp>,
    /// `None` while the plan it ran is not readable, or past the year 9999.
    pub deadline: Option<Timestamp>,
    pub items: ItemCounts,
}

/// How many of a run's task items stand in each [`ItemState`], and in all.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ItemCounts {
    pub total: usize,
    pub todo: usize,
    pub doing: usize,
    pub review: usize,
    pub done: usize,
    pub blocked: usize,
}

impl ItemCounts {
    /// `items` counted by state.
    pub fn of(items: &[RunItem]) -> Self {
        let mut counts = Self {
            total: items.len(),
            ..Self::default()
        };
        for item in items {
            let count = match item.state {
                ItemState::Todo => &mut counts.todo,
                ItemState::Doing => &mut counts.doing,
                ItemState::Review => &mut counts.review,
                ItemState::Done => &mut counts.done,
                ItemState::Blocked => &mut counts.blocked,
            };
            *count += 1;
        }
        assert_eq!(
            counts.todo + counts.doing + counts.review + counts.done + counts.blocked,
            counts.total,
            "every task item counts in exactly one state"
        );
        counts
    }
}

/// The summary of `shown`, a task as `show` reads it.
pub fn summary_of(shown: &ShownTask) -> TaskSummary {
    let plan = shown
        .plans
        .iter()
        .filter(|plan| is_current(plan.fields().state))
        .max_by_key(|plan| plan.fields().version)
        .map(|plan| PlanSummary {
            version: plan.fields().version,
            state: plan.fields().state,
        });
    TaskSummary {
        plan,
        run: shown.latest_run().map(|run| run_summary(run, &shown.plans)),
        may_be_incomplete: shown.may_be_incomplete(),
    }
}

/// The plan states a summary names (an allowlist): the one a planner
/// edits and the one a start runs.
fn is_current(state: PlanState) -> bool {
    match state {
        PlanState::Draft | PlanState::Accepted => true,
        PlanState::Superseded | PlanState::Discarded => false,
    }
}

/// `run` in brief, its deadline from the plan of `plans` it ran.
fn run_summary(run: &Run, plans: &[Plan]) -> RunSummary {
    let fields = run.fields();
    // The plan it ran, by the board's own rule: an accepted (or since
    // superseded) version of the task's plan holding each of its items.
    let ran = plans.iter().find(|plan| run.ran(plan).is_ok());
    let deadline = ran.and_then(|plan| {
        let seconds = deadline_seconds(plan.fields().team.as_ref());
        fields.started.plus_seconds(seconds).ok()
    });
    RunSummary {
        number: fields.number,
        plan_version: fields.plan_version,
        outcome: fields.outcome,
        started: fields.started.clone(),
        ended: fields.ended.clone(),
        deadline,
        items: ItemCounts::of(&fields.items),
    }
}

/// A listing with each task's summary, all from one read of the board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SummarizedListing {
    listing: BoardListing,
    summaries: Vec<TaskSummary>,
}

impl SummarizedListing {
    /// `listing`, each of its tasks summarised by `summaries`, in order.
    ///
    /// # Panics
    /// A summary count other than the listing's task count.
    pub fn new(listing: BoardListing, summaries: Vec<TaskSummary>) -> Self {
        assert_eq!(
            listing.tasks.len(),
            summaries.len(),
            "one summary per listed task"
        );
        Self { listing, summaries }
    }

    /// The head the listing and its summaries were read at.
    pub fn head(&self) -> &BoardHead {
        &self.listing.head
    }

    /// Each listed task, by id, beside its summary.
    pub fn entries(&self) -> impl Iterator<Item = (&Task, &TaskSummary)> {
        self.listing.tasks.iter().zip(&self.summaries)
    }

    /// Every file the board set aside.
    pub fn quarantined(&self) -> &[QuarantinedFile] {
        &self.listing.quarantined
    }

    /// The listing without its summaries.
    pub fn listing(&self) -> &BoardListing {
        &self.listing
    }
}

#[cfg(test)]
#[path = "task_summary_tests.rs"]
mod tests;
