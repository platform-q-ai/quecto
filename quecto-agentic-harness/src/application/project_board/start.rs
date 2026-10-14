//! What a start comes to (S4c): the task started, its run, and the
//! coordinator launched for it; and what it saw, for its record.
use super::ports::LaunchedCoordinator;
use super::swarm_brief::NotApplied;
use super::telemetry::OpFacts;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::entities::task::Task;
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

/// What a start saw (S4c), for its `board_start` record: gathered only for
/// a recorded start, and only ever counts and kinds. Each fact is `None`
/// where the start did not get that far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StartFacts {
    /// The start's own write: its attempts, lost races, and the task's
    /// status as last read and as written.
    pub op: OpFacts,
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
}
