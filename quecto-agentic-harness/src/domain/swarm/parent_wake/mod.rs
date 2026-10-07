//! Whether a swarm coordinator's idle turn should wake its parent (#2467):
//! decided from the run's board, so the parent hears when the phase finished,
//! went idle with nothing in flight, or could not be read, and not on every
//! idle turn the coordinator takes while its workers are busy.

/// The run as its coordinator's harness reads it at an idle boundary: the
/// run status and the tasks and workers that can still make progress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoordinatorBoard {
    /// The run's status as the board stores it (`running`, `complete`, ...).
    pub status: String,
    /// Tasks ready to claim (dependencies met).
    pub ready: i64,
    /// Tasks a worker holds.
    pub claimed: i64,
    /// Live workers, the coordinator excluded, holding no task.
    pub idle_workers: i64,
}

/// What the coordinator's parent should hear at this idle boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParentWake {
    /// Workers are busy or can take ready work: say nothing.
    Hold,
    /// The run left `running` (complete, stopped, paused, failed, lost...):
    /// its final report, or the reason it stopped, is ready.
    Finished { status: String },
    /// The run is running with nothing claimed and no worker to take ready
    /// work: the coordinator needs a decision or is stuck.
    Idle,
    /// The board could not be read: wake as an ordinary turn end would.
    Unknown,
}

/// The run status in which a coordinator's idle turn can be held.
const RUNNING: &str = "running";

/// Decide the wake for `board` (`None` when it could not be read).
pub fn parent_wake(board: Option<&CoordinatorBoard>) -> ParentWake {
    let _ = (board, RUNNING);
    ParentWake::Unknown
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
