//! Whether a swarm coordinator's idle turn should wake its parent (#2467):
//! decided from the run's board, so the parent hears when the phase finished,
//! went idle with nothing in flight, or could not be read, and not on every
//! idle turn the coordinator takes while its workers are busy.

use super::RunStatus;

/// The run as its coordinator's harness reads it at an idle boundary: the
/// run status and the tasks and workers that can still make progress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoordinatorBoard {
    /// The run's status as the board stores it.
    pub status: RunStatus,
    /// The result a run that ended holds (`paused` with an outcome is how
    /// the board ends a run), `None` while it has none.
    pub outcome: Option<RunStatus>,
    /// Tasks ready to claim (dependencies met).
    pub ready: i64,
    /// Tasks a worker holds.
    pub claimed: i64,
    /// Tasks a worker submitted for the coordinator to accept or reject.
    pub submitted: i64,
    /// Live workers, the coordinator excluded, holding no task.
    pub idle_workers: i64,
}

/// What the coordinator's parent should hear at this idle boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParentWake {
    /// Work is in flight (claimed, submitted, or ready for a free worker):
    /// say nothing. `status` is the run's (`setup` or `running`).
    Hold { status: RunStatus },
    /// The run ended with `outcome`: its final report is ready.
    Finished { outcome: RunStatus },
    /// Nothing in flight and no result (`status` is `setup` or `running`):
    /// the coordinator needs a decision or is stuck.
    Idle { status: RunStatus },
    /// The run is paused with no outcome, whatever is claimed: someone must
    /// resume or end it.
    Paused,
    /// The board could not be read: wake as an ordinary turn end would.
    Unknown,
}

/// The wake's kind as it travels from the coordinator to its parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WakeKind {
    Hold,
    Finished,
    Idle,
    Paused,
    /// The board could not be read, or a kind this build does not know.
    #[serde(other)]
    Unknown,
}

impl ParentWake {
    pub fn kind(&self) -> WakeKind {
        match self {
            Self::Hold { .. } => WakeKind::Hold,
            Self::Finished { .. } => WakeKind::Finished,
            Self::Idle { .. } => WakeKind::Idle,
            Self::Paused => WakeKind::Paused,
            Self::Unknown => WakeKind::Unknown,
        }
    }

    /// The run status the wake names: the outcome of a finished run, the
    /// live status otherwise, none when the board could not be read.
    pub fn status(&self) -> Option<RunStatus> {
        match self {
            Self::Hold { status } | Self::Idle { status } => Some(*status),
            Self::Finished { outcome } => Some(*outcome),
            Self::Paused => Some(RunStatus::Paused),
            Self::Unknown => None,
        }
    }
}

/// Decide the wake for `board` (`None` when it could not be read).
pub fn parent_wake(board: Option<&CoordinatorBoard>) -> ParentWake {
    let Some(board) = board else {
        return ParentWake::Unknown;
    };
    match (board.status, board.outcome) {
        (RunStatus::Setup | RunStatus::Running, _) => in_flight(board),
        (RunStatus::Paused, Some(outcome)) => ParentWake::Finished { outcome },
        (RunStatus::Paused, None) => ParentWake::Paused,
        (
            RunStatus::Succeeded
            | RunStatus::Blocked
            | RunStatus::Failed
            | RunStatus::Cancelled
            | RunStatus::BudgetExhausted,
            _,
        ) => ParentWake::Finished {
            outcome: board.status,
        },
    }
}

/// A live run holds while any work is in flight.
fn in_flight(board: &CoordinatorBoard) -> ParentWake {
    let status = board.status;
    let workers_busy = board.claimed > 0;
    let awaiting_verdict = board.submitted > 0;
    let ready_work_taken = board.ready > 0 && board.idle_workers > 0;
    match workers_busy || awaiting_verdict || ready_work_taken {
        true => ParentWake::Hold { status },
        false => ParentWake::Idle { status },
    }
}

/// What a coordinator's held turn-end note becomes once its run's state
/// arrives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeldNote {
    /// Work is in flight and nobody is waiting on this turn: no note.
    Silent,
    /// The turn-end note the parent would have had anyway.
    Ordinary,
    /// The run ended: name its outcome.
    Finished,
    /// Nothing in flight and no result: the parent may need to decide.
    Idle,
    /// The run is paused with no result: the parent may need to decide.
    Paused,
    /// The coordinator's last turn failed with work in flight or the board
    /// unreadable: it may need a resume.
    Stopped,
}

/// How the idle stretch a wake closes ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StretchEnd {
    /// A client's prompt (the parent's question, say) ran in it: its reply
    /// is due even while workers are busy.
    pub prompted: bool,
    /// Its last turn failed: the coordinator is stopped until resumed.
    pub errored: bool,
}

/// Settle a held turn end (or a failed last turn) by the run's wake.
pub fn settle_held_note(kind: WakeKind, end: StretchEnd) -> HeldNote {
    match (kind, end.prompted, end.errored) {
        (WakeKind::Finished, _, _) => HeldNote::Finished,
        (WakeKind::Idle, _, _) => HeldNote::Idle,
        (WakeKind::Paused, _, _) => HeldNote::Paused,
        (WakeKind::Hold | WakeKind::Unknown, _, true) => HeldNote::Stopped,
        (WakeKind::Hold, false, false) => HeldNote::Silent,
        (WakeKind::Hold, true, false) | (WakeKind::Unknown, _, false) => HeldNote::Ordinary,
    }
}

/// Work stranded once every worker of a run is idle (#2471): tasks a
/// worker still holds, and ready work no free worker can take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkersStalled {
    pub claimed: i64,
    pub ready: i64,
}

/// Whether a run whose workers have all ended their turns is stalled: it is
/// live and a task is still claimed, or ready work has no free worker. Work
/// submitted or blocked already woke the coordinator through its board.
pub fn workers_stalled(board: &CoordinatorBoard) -> Option<WorkersStalled> {
    let live = matches!(board.status, RunStatus::Setup | RunStatus::Running);
    let claimed = board.claimed > 0;
    let ready_untaken = board.ready > 0 && board.idle_workers == 0;
    let stalled = WorkersStalled {
        claimed: board.claimed,
        ready: board.ready,
    };
    match (live, claimed || ready_untaken) {
        (true, true) => Some(stalled),
        (true, false) | (false, _) => None,
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
