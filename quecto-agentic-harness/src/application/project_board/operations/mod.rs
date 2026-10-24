//! The board operations every front end serves (#2477): one typed request
//! ([`BoardRequest`]) that the
//! [`RunBoardOperation`](super::use_cases::RunBoardOperation) use case runs
//! over the board's use cases and listings (composition builds and injects
//! them, [`BoardOperationParts`]) into one typed outcome. The front ends
//! (the `quecto board` CLI, the agent `board` tool) read and answer JSON
//! in the interface layer; here nothing is JSON.
use std::sync::Arc;

pub use super::dto::NoPullRequestHost;
use super::dto::{BoardListing, ProjectBoardError};
pub use super::dto::{
    BoardOutcome, BoardRequest, Completion, ListFilter, ListSummaries, PlanPage, PlanView,
};
use super::ports::{BoardClock, BoardListings, BoardStoreError};
use super::use_cases::{
    AcceptPlan, BootstrapProjectBoard, ChangeBoardTask, ClearSetAside, CompleteMergedTask,
    CreateBoardTask, DiscardPlan, EditPlan, EndRun, ListLeftRuns, ProposePlan, QueryBoard,
    StartBoardTask, UpdateDraftTask,
};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

/// `BoardListings` over `QueryBoard`, and the operations assembled as
/// composition assembles them, for the front ends' tests.
#[cfg(test)]
#[path = "query_listings_tests.rs"]
pub(crate) mod query_listings;

/// [`BoardListings`] read from the store itself through [`QueryBoard`]:
/// the meaning the index's contract holds it to, for a store with no
/// index (a board server's, epic #2705 B7; the tests' in-memory one).
pub struct StoreListings(pub QueryBoard);

/// A refusal as a store's error: its own, or its words.
fn store_error(error: ProjectBoardError) -> BoardStoreError {
    match error {
        ProjectBoardError::Store(error) => error,
        other => BoardStoreError::Unavailable(other.to_string()),
    }
}

impl BoardListings for StoreListings {
    fn ready(&self) -> Result<BoardListing, BoardStoreError> {
        self.0.ready().map_err(store_error)
    }

    fn takeable(&self, now: &Timestamp) -> Result<BoardListing, BoardStoreError> {
        self.0.takeable(now).map_err(store_error)
    }

    fn all(&self) -> Result<BoardListing, BoardStoreError> {
        self.0.all().map_err(store_error)
    }

    fn by_status(&self, status: TaskStatus) -> Result<BoardListing, BoardStoreError> {
        self.0.by_status(status).map_err(store_error)
    }

    fn children(&self, parent: &Slug) -> Result<BoardListing, BoardStoreError> {
        self.0.children(parent).map_err(store_error)
    }

    fn dependents(&self, task: &Slug) -> Result<BoardListing, BoardStoreError> {
        self.0.dependents(task).map_err(store_error)
    }
}

/// The use cases and ports one run of [`RunBoardOperation`] is made of,
/// built by composition over one board.
///
/// [`RunBoardOperation`]: super::use_cases::RunBoardOperation
pub struct BoardOperationParts {
    pub clock: Arc<dyn BoardClock>,
    pub listings: Arc<dyn BoardListings>,
    /// What the listings (the index) do not hold yet: a task with its
    /// plans and runs, read from the store's snapshot (#2481 M4a; S12 may
    /// answer it from the index).
    pub query: QueryBoard,
    pub bootstrap: BootstrapProjectBoard,
    pub create: CreateBoardTask,
    pub update: UpdateDraftTask,
    pub change: ChangeBoardTask,
    pub propose_plan: ProposePlan,
    pub edit_plan: EditPlan,
    pub accept_plan: AcceptPlan,
    pub discard_plan: DiscardPlan,
    pub end_run: EndRun,
    pub start: StartBoardTask,
    /// The runs this front end's starts launched that it would leave open
    /// as its session ends (#2655); `None` for one that launches none.
    pub left_runs: Option<ListLeftRuns>,
    pub clear: ClearSetAside,
    pub complete: CompleteMergedTask,
}
