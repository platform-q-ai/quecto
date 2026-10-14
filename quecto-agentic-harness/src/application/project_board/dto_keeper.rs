//! What a started run's keeper (S4d) is told of the swarm an environment
//! hosts: the answer of the
//! [`StartedSwarm`](super::super::ports::StartedSwarm) port.
use crate::application::project_board::fold_back::SwarmRunView;

/// What the session's live board view answers of the swarm an environment
/// hosts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwarmBoardSight {
    /// Its board, as seen now.
    Board(SwarmSight),
    /// The environment is gone: no longer this session's to read, stopped
    /// or torn down. It will not come back.
    Gone,
    /// The board could not be read this time, why.
    Unreadable(String),
}

/// A swarm's board as a keeper sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwarmSight {
    /// The run the board holds, by its id; `None` before the coordinator
    /// created one.
    pub run_id: Option<String>,
    /// Whether its coordinator is a live member.
    pub coordinator_alive: bool,
    /// Whether the view left rows out (past the board's limits).
    pub truncated: bool,
    pub stage: SwarmStage,
}

/// Where a swarm's run stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SwarmStage {
    /// Going (or not created yet).
    Going,
    /// Paused, waiting for its parent to close it; with the view its
    /// proposed end would fold back, when it proposes one.
    Paused { proposed: Option<SwarmRunView> },
    /// Ended, as it stood then.
    Ended(SwarmRunView),
}
