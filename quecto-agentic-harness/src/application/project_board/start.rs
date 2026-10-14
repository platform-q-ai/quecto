//! What a start comes to (S4c): the task started, its run, and the
//! coordinator launched for it; and what it saw, for its record.
use super::dto::{FoldBackFacts, FoldedBack, ProjectBoardError};
use super::ports::LaunchedCoordinator;
use super::swarm_brief::NotApplied;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::StartClaim;

/// A task started: as written, with its run, the coordinator launched for
/// it, the worker roles whose models were not applied, and whether the
/// start took the claim (`fresh`) or found it held by the starter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedTask {
    pub task: Task,
    pub run: Run,
    pub coordinator: LaunchedCoordinator,
    pub not_applied: Vec<NotApplied>,
    pub claim: StartClaim,
}

/// What a start saw (S4c), for its `board_start` record, beside the
/// `board_op` facts of its write ([`OpFacts`](super::telemetry::OpFacts),
/// which holds these as `start`): gathered only for a recorded start, and
/// only ever counts and kinds. Each fact is `None` where the start did
/// not get that far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StartFacts {
    pub run: Option<u64>,
    pub plan_version: Option<u64>,
    /// The member limit, the coordinator included.
    pub members: Option<u64>,
    pub roles: Option<u64>,
    pub items: Option<u64>,
    pub not_applied: Option<u64>,
    pub claim: Option<StartClaim>,
    /// How long the launch took, on the board clock's monotonic time.
    pub launch_us: Option<u64>,
    /// For a start undone: whether its own fold-back ended the run, and,
    /// when it took the claim, whether it released it.
    pub folded: Option<bool>,
    pub released: Option<bool>,
    /// The undo's fold-back, for its own `board_fold_back` record.
    pub fold_back: Option<UndoFoldBack>,
}

/// A start's undo fold-back (S4c), as its record is made from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoFoldBack {
    pub task: Slug,
    pub run: u32,
    pub result: Result<FoldedBack, ProjectBoardError>,
    pub facts: FoldBackFacts,
    pub elapsed: std::time::Duration,
}
