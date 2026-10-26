//! Which artifact a board file holds, by its kind's key (#2481), moved
//! beside `ports.rs` as it grew past its room (#2705 B9).
use crate::domain::project_board::entities::change::Change;
use crate::domain::project_board::entities::discussion::Discussion;
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::entities::review::Review;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::slug::Slug;

/// Which artifact a board file holds (#2481's artifact model): what names
/// it on the board, one file each. A task by its id; a plan, run or change
/// by its task and number; a review by its task and its change's number; a
/// discussion by its id.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ArtifactKey {
    Task(Slug),
    Plan(Slug, u32),
    Run(Slug, u32),
    Change(Slug, u32),
    Review(Slug, u32),
    Discussion(Slug),
}

impl ArtifactKey {
    pub fn of_task(task: &Task) -> Self {
        Self::Task(task.fields().id.clone())
    }

    pub fn of_plan(plan: &Plan) -> Self {
        Self::Plan(plan.fields().task.clone(), plan.fields().version)
    }

    pub fn of_run(run: &Run) -> Self {
        Self::Run(run.fields().task.clone(), run.fields().number)
    }

    pub fn of_change(change: &Change) -> Self {
        Self::Change(change.fields().task.clone(), change.fields().number)
    }

    pub fn of_review(review: &Review) -> Self {
        Self::Review(review.fields().task.clone(), review.fields().change)
    }

    pub fn of_discussion(discussion: &Discussion) -> Self {
        Self::Discussion(discussion.fields().id.clone())
    }

    /// The task's id, when it is a task's own key (not the task a plan,
    /// run, change or review belongs to).
    pub fn as_task(&self) -> Option<&Slug> {
        match self {
            Self::Task(id) => Some(id),
            Self::Plan(..)
            | Self::Run(..)
            | Self::Change(..)
            | Self::Review(..)
            | Self::Discussion(_) => None,
        }
    }
}

/// How a message names it: `task x`, `plan 2 of task x`, `review 3 of
/// task x` (the review of its change 3), `discussion d`.
impl std::fmt::Display for ArtifactKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Task(id) => write!(f, "task {}", id.as_str()),
            Self::Plan(id, version) => write!(f, "plan {version} of task {}", id.as_str()),
            Self::Run(id, number) => write!(f, "run {number} of task {}", id.as_str()),
            Self::Change(id, number) => write!(f, "change {number} of task {}", id.as_str()),
            Self::Review(id, change) => write!(f, "review {change} of task {}", id.as_str()),
            Self::Discussion(id) => write!(f, "discussion {}", id.as_str()),
        }
    }
}
