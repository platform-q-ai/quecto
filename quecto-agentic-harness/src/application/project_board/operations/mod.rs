//! The board operations every front end serves (#2477): one typed request
//! ([`BoardRequest`]) that the
//! [`RunBoardOperation`](super::use_cases::RunBoardOperation) use case runs
//! over the board's use cases and listings (composition builds and injects
//! them, [`BoardOperationParts`]) into one typed outcome. The front ends
//! (the `quecto board` CLI, the agent `board` tool) read and answer JSON
//! in the interface layer; here nothing is JSON.
use std::sync::Arc;

pub use super::dto::NoPullRequestHost;
pub use super::dto::{
    BoardOutcome, BoardRequest, Completion, ListFilter, ListSummaries, PlanPage, PlanView,
};
use super::ports::{BoardClock, BoardListings};
use super::use_cases::{
    AcceptPlan, BootstrapProjectBoard, ChangeBoardTask, ClearSetAside, CompleteMergedTask,
    CreateBoardTask, DiscardPlan, EditPlan, EndRun, FinishWithoutRun, ListLeftRuns, ProposePlan,
    QueryBoard, StartBoardTask, UpdateDraftTask,
};

/// `BoardListings` over `QueryBoard`, and the operations assembled as
/// composition assembles them, for the front ends' tests.
#[cfg(test)]
#[path = "query_listings_tests.rs"]
pub(crate) mod query_listings;

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
    /// A task done without a run, its linked pull requests read as a
    /// completion reads them (#2705 T4).
    pub done_without_run: FinishWithoutRun,
}
