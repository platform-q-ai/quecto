//! What a start comes to (S4c): the task started, its run, and the
//! coordinator launched for it; and what it saw, for its record. A start
//! across the wire (epic #2705 B8) is begun on the board, launched by its
//! client, and undone on the board when its launch fails.
use super::dto::{FoldBackFacts, FoldedBack, ProjectBoardError};
use super::ports::{CoordinatorRequest, LaunchedCoordinator};
use super::swarm_brief::NotApplied;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::services::starts::Started;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::telemetry::StartClaim;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

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
    /// For a start undone: where its undo left the run, and, when it took
    /// the claim, whether it released it.
    pub run_state: Option<super::dto::RunAfterUndo>,
    pub released: Option<bool>,
    /// The undo's fold-back, for its own `board_fold_back` record.
    pub fold_back: Option<NotedFoldBack>,
}

/// A fold-back a request made, as its record is made from it: a start's
/// undo (S4c), or an `end_run`'s (#2667).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotedFoldBack {
    pub task: Slug,
    pub run: u32,
    pub result: Result<FoldedBack, ProjectBoardError>,
    pub facts: FoldBackFacts,
    pub elapsed: std::time::Duration,
}

/// A start begun (epic #2705 B8): its task and run written, its
/// coordinator not launched yet. `written` is the start as its board
/// answered it: written on this machine ([`StartWrite`]), or a board
/// server's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Begun<W> {
    /// What the coordinator is launched with: the task, the run, the brief.
    pub launch: CoordinatorRequest,
    /// The status the start took the task from: its undo puts a task
    /// taken from review or blocked back there (#2705 T5).
    pub from: TaskStatus,
    pub claim: StartClaim,
    /// What the begin saw, for the start's record: counts and kinds only.
    pub noted: StartFacts,
    pub written: W,
}

impl<W> Begun<W> {
    /// The undo of this start, whose coordinator did not launch for `reason`.
    pub fn undo(&self, reason: &str) -> StartUndo {
        StartUndo {
            task: self.launch.task.clone(),
            plan_version: self.launch.plan_version,
            run: self.launch.run,
            from: self.from,
            reason: reason.to_string(),
        }
    }
}

/// A start as this machine's board wrote it, and the worker roles whose
/// models were not applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartWrite {
    pub started: Started,
    pub not_applied: Vec<NotApplied>,
}

/// The undo of a begun start whose coordinator did not launch (B8): run
/// `run` of `task`, begun on plan `plan_version` from status `from`,
/// folded back as failed, and why the launch failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartUndo {
    pub task: Slug,
    pub plan_version: u32,
    pub run: u32,
    pub from: TaskStatus,
    pub reason: String,
}
