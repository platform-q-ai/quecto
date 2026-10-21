//! The swarm runs an environment hosts: their observation, live and ended
//! boards, and the inspection the restore uses.

use super::PortFuture;
use crate::application::environments::dto::HostedBoardRevision;
use crate::application::environments::dto::SwarmBoardSnapshot;
use crate::domain::environments::entities::environment_registry::EnvironmentRecord;
use crate::domain::environments::services::environment_retention::CompletionDelivery;
use crate::domain::environments::services::environment_retention::CoordinatorLoss;
use crate::domain::environments::services::environment_retention::HostedSwarmRun;
use crate::domain::environments::services::environment_retention::SwarmRunObservation;
use std::path::Path;

/// What the supervising session can learn about — and record on — the swarm
/// run an environment hosts (#1924).
pub trait HostedSwarmRunObservation: Send + Sync {
    /// Observe the swarm run `record` hosts, when it advertises a
    /// coordination store this session can reach.
    fn observe_hosted_swarm_run<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, SwarmRunObservation>;

    /// Record the coordinator's harness as lost on the hosted run in ONE
    /// store operation: a run that has already ended (including one the
    /// store's own expiry check ends first) is left alone; any other run is
    /// paused holding `failed` (the #1729 lost-harness rule). Returns the run
    /// as it stands afterwards.
    fn record_lost_coordinator<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
        hosted: &'a HostedSwarmRun,
    ) -> PortFuture<'a, Result<CoordinatorLoss, String>>;
}

/// The board a swarm environment hosts, read by the supervising session
/// through the board's own adapter (#2478): no client opens the file.
pub trait HostedSwarmBoard: Send + Sync {
    /// The board's projection (the `_board` read), or `None` when
    /// `record` hosts no board this session can reach.
    fn board<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<SwarmBoardSnapshot>, String>>;

    /// The board's revision and the run it counts, or `None` as for
    /// [`Self::board`].
    fn revision<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, Result<Option<HostedBoardRevision>, String>>;
}

/// Where the board of a swarm environment is kept as its member leaves
/// (#2481 S4c follow-up): read before anything tears the box down, once
/// its run has ended, and kept under the session and the leaving member's
/// label (the name it was spawned with, `board-<task>-run-<n>` for a board
/// run's coordinator), so the session that launched that member can still
/// read how its run ended (its items, owners, evidence and spend) once
/// the box and its board are gone, with no memory of the launch.
pub trait EndedSwarmBoards: Send + Sync {
    /// Keeps `board`, the board the environment of `session`'s member
    /// `label` held as it left, in place of any kept for it before. Never
    /// fails the teardown: a board it cannot keep whole is not kept.
    fn keep(&self, session: &str, label: &str, board: SwarmBoardSnapshot);
}

/// Where the work of a succeeded swarm run went, read as its owner's kill
/// tears its box down (#2666 round 2): whether the commit its completion
/// names is on the board's remote. Blocking reads run inline, as the
/// teardown's scripts do.
pub trait CompletionDeliveryCheck: Send + Sync {
    /// For the run of member `label`, whose board as it left is `board`
    /// (`None`: it could not be read): `NotJudged` for a member that is no
    /// board run's coordinator, or a completion that names no commit;
    /// `Unknown` when the board, its completion or the remote cannot be
    /// read.
    fn delivery<'a>(
        &'a self,
        label: &'a str,
        board: Option<&'a SwarmBoardSnapshot>,
    ) -> PortFuture<'a, CompletionDelivery>;
}

/// The swarm run a checkout hosts, read synchronously and without a
/// session (round 4 M1, #2033): what [`HostedSwarmRunObservation`] learns
/// for the finalizer, for the startup restore and the CLI collector —
/// which run without a runtime and must judge a state directory the
/// registry knows nothing about. Read-only: nothing here records a loss.
pub trait HostedSwarmRunInspection: Send + Sync {
    /// The run `record`'s checkout hosts (its advertised checkout, else
    /// its workspace probed the way the observation port probes it).
    fn inspect_hosted_run(&self, record: &EnvironmentRecord) -> SwarmRunObservation;

    /// The run hosted below `state_dir` — an environment directory laid
    /// out by the shipped scripts (`<state_dir>/workspace[/repo]`) that no
    /// record names. The adapter owns the layout; a directory hosting no
    /// readable store is `NoStore`.
    fn inspect_hosted_run_at(&self, state_dir: &Path) -> SwarmRunObservation;
}
