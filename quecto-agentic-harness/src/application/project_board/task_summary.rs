//! What a listing says of each task it lists, when asked
//! ([`ListSummaries::Wanted`]): the `summary` beside each task of a
//! `list`, `list_ready` and `list_takeable` answer. It is read from one
//! read of the board, the one the task's row was listed at ([`ShownTask`],
//! as `show` reads it), so the two always agree.
//!
//! [`ListSummaries::Wanted`]: super::operations::ListSummaries::Wanted
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
use super::dto::{BoardListing, ProjectBoardError, ShownTask};
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

/// A listing as a listing op answers it: its tasks, and their summaries
/// as asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    listing: BoardListing,
    summaries: RowSummaries,
}

/// What a listing says of its tasks beyond their rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowSummaries {
    /// None was asked for.
    Omitted,
    /// One per listed task, in order, all read from the board the listing
    /// was read at.
    Given(Vec<TaskSummary>),
    /// Asked for, but not read: why (the board kept moving, or its read
    /// failed). The listing is answered all the same.
    Skipped(Box<ProjectBoardError>),
}

impl Listed {
    /// `listing` alone: no summary asked for.
    pub fn plain(listing: BoardListing) -> Self {
        Self {
            listing,
            summaries: RowSummaries::Omitted,
        }
    }

    /// `listing`, each of its tasks summarised by `summaries`, in order.
    ///
    /// # Panics
    /// A summary count other than the listing's task count.
    pub fn summarized(listing: BoardListing, summaries: Vec<TaskSummary>) -> Self {
        assert_eq!(
            listing.tasks.len(),
            summaries.len(),
            "one summary per listed task"
        );
        Self {
            listing,
            summaries: RowSummaries::Given(summaries),
        }
    }

    /// `listing`, its summaries skipped for `why`.
    pub fn skipped(listing: BoardListing, why: ProjectBoardError) -> Self {
        Self {
            listing,
            summaries: RowSummaries::Skipped(Box::new(why)),
        }
    }

    /// The head the listing was read at.
    pub fn head(&self) -> &BoardHead {
        &self.listing.head
    }

    /// Each listed task, by id, beside its summary while summaries were
    /// given.
    pub fn entries(&self) -> impl Iterator<Item = (&Task, Option<&TaskSummary>)> {
        let summaries = match &self.summaries {
            RowSummaries::Given(summaries) => Some(summaries),
            RowSummaries::Omitted | RowSummaries::Skipped(_) => None,
        };
        self.listing
            .tasks
            .iter()
            .enumerate()
            .map(move |(index, task)| (task, summaries.map(|summaries| &summaries[index])))
    }

    /// What it says of its tasks beyond their rows.
    pub fn summaries(&self) -> &RowSummaries {
        &self.summaries
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
