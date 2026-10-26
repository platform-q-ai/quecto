//! [`BoardListings`] over `QueryBoard` for tests (#2477): the index's
//! meaning (its contract holds it to `QueryBoard`), on any store, such as
//! the in-memory one; and [`assembled`], the operations over a store as
//! composition assembles them.
use std::sync::Arc;

use super::{BoardOperationParts, NoPullRequestHost};
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{
    BoardClock, BoardListings, CoordinatorLaunch, ProjectBoardStore, PullRequestStates,
};
use crate::application::project_board::use_cases::{
    AcceptPlan, BootstrapProjectBoard, ChangeBoardTask, ClearSetAside, CompleteMergedTask,
    CreateBoardTask, DiscardPlan, EditPlan, FinishWithoutRun, ProposePlan, QueryBoard,
    RunBoardOperation, StartBoardTask, UpdateDraftTask,
};
use crate::infrastructure::persistence::project_board::text_cells::MarkdownTextCells;

pub use super::StoreListings as QueryListings;

/// The operations over `store`, its `listings` and `clock`, with pull
/// requests read from `prs` (or none, and why), as composition builds
/// them.
pub fn assembled(
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    listings: Arc<dyn BoardListings>,
    prs: Result<Arc<dyn PullRequestStates>, NoPullRequestHost>,
) -> Result<RunBoardOperation, ProjectBoardError> {
    assembled_launching(store, clock, listings, prs, None, None)
}

/// [`assembled`], starting tasks with `launcher` and seeing runs' swarms
/// through `liveness` (S4c).
pub fn assembled_launching(
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    listings: Arc<dyn BoardListings>,
    prs: Result<Arc<dyn PullRequestStates>, NoPullRequestHost>,
    launcher: Option<Arc<dyn CoordinatorLaunch>>,
    liveness: Option<Arc<dyn crate::application::project_board::ports::RunLiveness>>,
) -> Result<RunBoardOperation, ProjectBoardError> {
    assembled_reporting(
        store,
        clock,
        listings,
        prs,
        Swarms {
            launcher,
            started: None,
            liveness,
            reports: None,
        },
    )
}

/// What a front end reaches a session's swarms through: the launcher, the
/// liveness of runs' swarms and how they ended (#2481 S4c follow-up).
pub struct Swarms {
    pub launcher: Option<Arc<dyn CoordinatorLaunch>>,
    /// The runs the launcher started (#2655).
    pub started: Option<Arc<dyn crate::application::project_board::ports::StartedRuns>>,
    pub liveness: Option<Arc<dyn crate::application::project_board::ports::RunLiveness>>,
    pub reports: Option<Arc<dyn crate::application::project_board::ports::SwarmRunReports>>,
}

/// [`assembled_launching`], reading how runs' swarms ended through
/// `swarms.reports` too.
pub fn assembled_reporting(
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    listings: Arc<dyn BoardListings>,
    prs: Result<Arc<dyn PullRequestStates>, NoPullRequestHost>,
    swarms: Swarms,
) -> Result<RunBoardOperation, ProjectBoardError> {
    let Swarms {
        launcher,
        started,
        liveness,
        reports,
    } = swarms;
    let done_without_run =
        FinishWithoutRun::reading(Arc::clone(&store), Arc::clone(&clock), prs.clone())?;
    let complete = CompleteMergedTask::reading(Arc::clone(&store), Arc::clone(&clock), prs)?;
    Ok(RunBoardOperation::new(BoardOperationParts {
        bootstrap: BootstrapProjectBoard::new(Arc::clone(&store)),
        create: CreateBoardTask::new(
            Arc::clone(&store),
            Arc::clone(&clock),
            Arc::new(MarkdownTextCells),
        )?,
        update: UpdateDraftTask::new(
            Arc::clone(&store),
            Arc::clone(&clock),
            Arc::new(MarkdownTextCells),
        )?,
        change: ChangeBoardTask::new(Arc::clone(&store), Arc::clone(&clock))?,
        propose_plan: ProposePlan::new(Arc::clone(&store), Arc::clone(&clock))?,
        edit_plan: EditPlan::new(Arc::clone(&store), Arc::clone(&clock))?,
        accept_plan: AcceptPlan::new(Arc::clone(&store), Arc::clone(&clock))?,
        discard_plan: DiscardPlan::new(Arc::clone(&store), Arc::clone(&clock))?,
        end_run: crate::application::project_board::use_cases::EndRun::new(
            Arc::clone(&store),
            Arc::clone(&clock),
            Arc::new(MarkdownTextCells),
            crate::application::project_board::use_cases::fixtures_tests::Remote::holding_all(),
            liveness,
            reports,
        )?,
        start: super::Starting::Launched(StartBoardTask::new(
            Arc::clone(&store),
            Arc::clone(&store),
            Arc::clone(&clock),
            Arc::new(MarkdownTextCells),
            launcher,
            None,
        )?),
        left_runs: started.map(|started| {
            crate::application::project_board::use_cases::ListLeftRuns::new(
                Arc::clone(&store),
                started,
            )
        }),
        clear: ClearSetAside::new(Arc::clone(&store), Arc::clone(&clock))?,
        complete,
        done_without_run,
        clock,
        listings,
        query: QueryBoard::new(store),
    }))
}
