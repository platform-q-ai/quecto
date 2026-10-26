//! The board's listings, read from the store's snapshot: ready tasks,
//! takeable tasks, tasks by status, a task's children and its dependents,
//! each by id, and a task's plans, runs, changes, review and discussions,
//! each beside every file the board set aside; and a task as `show` gives
//! it, with its plans and runs (#2481 M4a). (S1e's index may answer them
//! faster later, behind this use case.)
use std::sync::Arc;

use super::board_view;
use crate::application::project_board::dto::{
    ArtifactListing, ArtifactScope, BoardListing, ProjectBoardError, ShownTask,
};
use crate::application::project_board::ports::{
    ArtifactKey, BoardHead, BoardSnapshot, ProjectBoardStore,
};
use crate::application::project_board::task_summary::{RowSummary, Summarized, summary_of};
use crate::domain::project_board::entities::change::Change;
use crate::domain::project_board::entities::discussion::Discussion;
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::entities::review::Review;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::claims::is_live;
use crate::domain::project_board::services::transitions::relations_allow;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

pub struct QueryBoard {
    store: Arc<dyn ProjectBoardStore>,
}

impl QueryBoard {
    pub fn new(store: Arc<dyn ProjectBoardStore>) -> Self {
        Self { store }
    }

    /// Ready tasks whose every dependency is on the board and releases
    /// its dependents ([`TaskStatus::releases_dependents`]: done or
    /// archived): the ones anyone may start now. (`by_status(Ready)` lists
    /// them all.)
    ///
    /// # Errors
    /// The store's.
    pub fn ready(&self) -> Result<BoardListing, ProjectBoardError> {
        // The claim rule itself (#2661): a ready task its dependencies let
        // anyone claim.
        self.matching(|task, board| {
            task.fields().status == TaskStatus::Ready
                && relations_allow(task, TaskStatus::Claimed, board).is_ok()
        })
    }

    /// Held tasks whose claim expired by `now`: anyone may take them over.
    ///
    /// # Errors
    /// The store's.
    pub fn takeable(&self, now: &Timestamp) -> Result<BoardListing, ProjectBoardError> {
        self.matching(|task, _| {
            let fields = task.fields();
            fields.status.may_hold_claim()
                && fields
                    .claim
                    .as_ref()
                    .is_some_and(|claim| !is_live(claim, now))
        })
    }

    /// # Errors
    /// The store's.
    pub fn all(&self) -> Result<BoardListing, ProjectBoardError> {
        self.matching(|_, _| true)
    }

    /// # Errors
    /// The store's.
    pub fn by_status(&self, status: TaskStatus) -> Result<BoardListing, ProjectBoardError> {
        self.matching(|task, _| task.fields().status == status)
    }

    /// The tasks naming `parent` as their parent.
    ///
    /// # Errors
    /// The store's.
    pub fn children(&self, parent: &Slug) -> Result<BoardListing, ProjectBoardError> {
        self.matching(|task, _| task.fields().parent.as_ref() == Some(parent))
    }

    /// The tasks that depend on `task`.
    ///
    /// # Errors
    /// The store's.
    pub fn dependents(&self, task: &Slug) -> Result<BoardListing, ProjectBoardError> {
        self.matching(|candidate, _| candidate.fields().depends_on.contains(task))
    }

    /// Every plan of `task`, by version, a plan of a task the board no
    /// longer holds included.
    ///
    /// # Errors
    /// The store's, or [`ProjectBoardError::Duplicate`] for an artifact in
    /// its scope the board holds twice.
    pub fn plans_of(&self, task: &Slug) -> Result<ArtifactListing<Plan>, ProjectBoardError> {
        ArtifactListing::plans_of(&self.store.read()?, task)
    }

    /// Every run of `task` of its plan's `version`, by number, whether or
    /// not that plan is on the board.
    ///
    /// # Errors
    /// The store's, or [`ProjectBoardError::Duplicate`] for an artifact in
    /// its scope the board holds twice.
    pub fn runs_of_plan(
        &self,
        task: &Slug,
        version: u32,
    ) -> Result<ArtifactListing<Run>, ProjectBoardError> {
        ArtifactListing::runs_of_plan(&self.store.read()?, task, version)
    }

    /// Every change of `task`, by number.
    ///
    /// # Errors
    /// The store's, or [`ProjectBoardError::Duplicate`] for an artifact in
    /// its scope the board holds twice.
    pub fn changes_of(&self, task: &Slug) -> Result<ArtifactListing<Change>, ProjectBoardError> {
        ArtifactListing::changes_of(&self.store.read()?, task)
    }

    /// The review of `task`'s change `change`, if the board holds one.
    ///
    /// # Errors
    /// The store's, or [`ProjectBoardError::Duplicate`] for an artifact in
    /// its scope the board holds twice.
    pub fn review_of(
        &self,
        task: &Slug,
        change: u32,
    ) -> Result<ArtifactListing<Review>, ProjectBoardError> {
        ArtifactListing::review_of(&self.store.read()?, task, change)
    }

    /// Every discussion linked to `task`, by id.
    ///
    /// # Errors
    /// The store's, or [`ProjectBoardError::Duplicate`] for an artifact in
    /// its scope the board holds twice.
    pub fn discussions_of(
        &self,
        task: &Slug,
    ) -> Result<ArtifactListing<Discussion>, ProjectBoardError> {
        ArtifactListing::discussions_of(&self.store.read()?, task)
    }

    /// The task `id` alone, as the board holds it.
    ///
    /// # Errors
    /// The store's; the task's file set aside, no task `id` on the board,
    /// or [`ProjectBoardError::Duplicate`] for a task held twice.
    pub fn task(&self, id: &Slug) -> Result<Task, ProjectBoardError> {
        Ok(board_view::task(&self.store.read()?, id)?.clone())
    }

    /// The task `id` with every plan and run of it, from one read of the
    /// board ([`ShownTask`]).
    ///
    /// # Errors
    /// The store's; the task's own file set aside, or no task `id` on the
    /// board; or [`ProjectBoardError::Duplicate`] for a task, plan or run
    /// of it the board holds twice. A plan or run file set aside is named
    /// in the answer, never refused.
    pub fn show(&self, id: &Slug) -> Result<ShownTask, ProjectBoardError> {
        shown_on(&self.store.read()?, id)
    }

    /// The plan `shown` gives for `version` as it was at `rev` (#2705
    /// B9): a page pinned to the revision of its first page.
    ///
    /// # Errors
    /// The plan changed since (`plan_changed`); the store's.
    pub fn page_as_read(
        &self,
        shown: &ShownTask,
        version: Option<u32>,
        rev: &BoardHead,
    ) -> Result<(), ProjectBoardError> {
        let plan = shown.plan(version);
        super::read_pins::page_as_read(&*self.store, &shown.head, plan, rev)
    }

    /// What one read of the whole board makes of the summaries of the
    /// tasks `listing` lists ([`summarized_on`]).
    ///
    /// # Errors
    /// The store's.
    pub fn summarized(&self, listing: &BoardListing) -> Result<Summarized, ProjectBoardError> {
        summarized_on(&self.store.read()?, listing)
    }

    fn matching(
        &self,
        wanted: impl Fn(&Task, &[Task]) -> bool,
    ) -> Result<BoardListing, ProjectBoardError> {
        let board = self.store.read()?;
        let mut tasks: Vec<Task> = board
            .tasks
            .iter()
            .filter(|task| wanted(task, &board.tasks))
            .cloned()
            .collect();
        tasks.sort_by(|left, right| left.fields().id.cmp(&right.fields().id));
        Ok(BoardListing {
            head: board.head,
            tasks,
            quarantined: board.quarantined,
        })
    }
}

/// The summaries of the tasks `listing` lists, in order ([`summary_of`]),
/// from `board`, each task read exactly as `show` reads it: none while
/// `board` is not where the listing was read ([`Summarized::Moved`]) or
/// holds another row than it lists, the task twice included
/// ([`Summarized::Disagrees`]), so a summary always matches its task's
/// row. A task one of whose plans or runs the board holds twice goes
/// without its own summary ([`RowSummary::Skipped`]); the others are
/// summarised.
///
/// # Errors
/// What reading a listed task refuses beyond a duplicate or a row the
/// board does not hold as listed: none `show`'s read gives today.
pub(crate) fn summarized_on(
    board: &BoardSnapshot,
    listing: &BoardListing,
) -> Result<Summarized, ProjectBoardError> {
    if listing.head != board.head {
        return Ok(Summarized::Moved);
    }
    let mut rows = Vec::with_capacity(listing.tasks.len());
    for task in &listing.tasks {
        let row = match shown_on(board, &task.fields().id) {
            Ok(shown) if shown.task == *task => RowSummary::Summary(Box::new(summary_of(&shown))),
            // The board, where the listing was read, holds another task by
            // its id, none it can read, or the task twice: the index
            // disagrees with it.
            Ok(_) | Err(ProjectBoardError::NotFound(_)) => return Ok(Summarized::Disagrees),
            Err(
                ProjectBoardError::Quarantined {
                    key: ArtifactKey::Task(_),
                    ..
                }
                | ProjectBoardError::Duplicate {
                    key: ArtifactKey::Task(_),
                },
            ) => return Ok(Summarized::Disagrees),
            // A plan or run of this task held twice: this row alone goes
            // without its summary.
            Err(
                error @ ProjectBoardError::Duplicate {
                    key: ArtifactKey::Plan(..) | ArtifactKey::Run(..),
                },
            ) => RowSummary::Skipped(Box::new(error)),
            Err(error) => return Err(error),
        };
        rows.push(row);
    }
    assert_eq!(rows.len(), listing.tasks.len(), "one per listed task");
    Ok(Summarized::Rows(rows))
}

/// [`QueryBoard::show`] over the snapshot `board`.
fn shown_on(board: &BoardSnapshot, id: &Slug) -> Result<ShownTask, ProjectBoardError> {
    let task = board_view::task(board, id)?.clone();
    let plans = ArtifactListing::plans_of(board, id)?;
    let runs = ArtifactListing::runs_of(board, id)?;
    let in_scope = |scope: &ArtifactScope, file| scope.holds(file);
    // The board's own order, each file once.
    let set_aside = board
        .quarantined
        .iter()
        .filter(|file| in_scope(&plans.scope, file) || in_scope(&runs.scope, file))
        .cloned()
        .collect();
    let ordered = |numbers: Vec<u32>| numbers.windows(2).all(|pair| pair[0] < pair[1]);
    assert!(
        ordered(
            plans
                .items
                .iter()
                .map(|plan| plan.fields().version)
                .collect()
        ) && ordered(runs.items.iter().map(|run| run.fields().number).collect()),
        "a task's plans by version and runs by number, each once"
    );
    Ok(ShownTask {
        head: board.head.clone(),
        task,
        plans: plans.items,
        runs: runs.items,
        set_aside,
    })
}

#[cfg(test)]
#[path = "query_board_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "query_board_artifacts_tests.rs"]
mod artifacts_tests;

#[cfg(test)]
#[path = "query_board_show_tests.rs"]
mod show_tests;

#[cfg(test)]
#[path = "query_board_summary_tests.rs"]
mod summary_tests;
