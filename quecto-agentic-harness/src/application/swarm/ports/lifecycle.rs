//! The swarm lifecycle ports (#1940, #1960, #2471): coordination, process
//! observation and control, the clock, the lifecycle entrypoint, supervisor run
//! control and the boards a wake reads.

use super::PortFuture;
use crate::domain::shared::error::DomainError;
use crate::domain::swarm::entities::run::{Member, Snapshot};
use crate::domain::swarm::services::parent_wake::CoordinatorBoard;
use crate::domain::swarm::services::worker_wake::WorkerBoard;
use crate::domain::swarm::value_objects::lifecycle::{MemberExit, ProcessIdentity, RunStatus};
use crate::domain::swarm::value_objects::run_control::{RunControlAction, RunControlReceipt};

/// Implementations atomically enforce membership policy before reserving slots.
/// Wire names, positional arguments and persistence schemas are private details.
pub trait CoordinationPort {
    fn snapshot(&self) -> Result<Snapshot, DomainError>;
    fn register_endpoint(&self, endpoint: &str) -> Result<(), DomainError>;
    fn reserve_member(&self, member: &str, token: &str) -> Result<(), DomainError>;
    fn record_launch(
        &self,
        member: &str,
        token: &str,
        process: &ProcessIdentity,
    ) -> Result<(), DomainError>;
    fn confirm_unlaunched(&self, member: &str) -> Result<(), DomainError>;
    /// A member's harness vanished without an authoritative exit observation
    /// (or the coordinator was lost, #1924): ownership is retained and the
    /// run pauses holding `failed`.
    fn quarantine(&self, member: &str) -> Result<(), DomainError>;
    /// The launching harness reaped the member's owned process (#1961): the
    /// member is dead, its active tasks block for the coordinator's
    /// `recover`, and the run keeps going. An orderly exit releases the
    /// member's file reservations; an abrupt one retains them.
    fn confirm_dead(&self, member: &str, exit: MemberExit) -> Result<(), DomainError>;
}

pub trait ProcessObservation {
    fn harness_dead(&self, process: &ProcessIdentity) -> bool;
}

pub trait ProcessControl: Sync {
    /// Suspend only this process; never signal a future turn or another member.
    fn suspend_local_inference(&self, snapshot: &Snapshot);
    fn abort<'a>(&'a self, member: &'a Member) -> PortFuture<'a, bool>;
    /// End the member's harness by delegation (#1939): the shutdown protocol
    /// over the endpoint it registered, the locally owned handle only when
    /// this harness launched it. The member's `ProcessIdentity` is an
    /// observation for liveness, never an authority to signal; a member
    /// reachable neither way is reported failed, not signalled.
    fn terminate<'a>(&'a self, member: &'a Member) -> PortFuture<'a, Result<(), DomainError>>;
}

pub trait Clock {
    fn now_seconds(&self) -> f64;
}

/// What a harness does next once its run has settled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettlementStep {
    /// Nothing is left for this harness: it is the coordinator, or its own
    /// row is no longer live, or the run is not terminal.
    Done,
    /// Still live: settle again on a fresh snapshot and wait for the launcher.
    Wait,
    /// Still live past the grace its launcher's teardown needs: end itself.
    EndSelf,
}

/// Application lifecycle entrypoint, injected by the composition root.
pub trait SwarmLifecycle: std::fmt::Debug + Send + Sync {
    fn reconcile(
        &self,
        coordination: &dyn CoordinationPort,
        processes: &dyn ProcessObservation,
    ) -> Result<Snapshot, DomainError>;
    /// An authoritative exit of a member this harness launched (#1961).
    fn member_exited(
        &self,
        coordination: &dyn CoordinationPort,
        processes: &dyn ProcessObservation,
        member: &str,
        exit: MemberExit,
    ) -> Result<Snapshot, DomainError>;
    fn settle<'a>(
        &'a self,
        snapshot: &'a Snapshot,
        actor: &'a str,
        processes: &'a dyn ProcessControl,
        observation: &'a (dyn ProcessObservation + Sync),
    ) -> PortFuture<'a, Result<(), DomainError>>;
    /// What a harness does next, `elapsed` after it first settled its run.
    fn settlement_step(
        &self,
        snapshot: &Snapshot,
        actor: &str,
        elapsed: std::time::Duration,
        grace: std::time::Duration,
    ) -> SettlementStep;
    /// A member still alive a grace after its run settled ends itself.
    fn settle_overdue<'a>(
        &'a self,
        snapshot: &'a Snapshot,
        actor: &'a str,
        processes: &'a dyn ProcessControl,
    ) -> PortFuture<'a, Result<(), DomainError>>;
    fn observed_outcome(&self, snapshot: &Snapshot, clock: &dyn Clock) -> RunStatus;
}

/// Supervisor operations remain available without model execution or turn-queue admission.
pub trait SwarmRunControl: Send + Sync {
    fn apply(
        &self,
        action: RunControlAction,
    ) -> PortFuture<'_, Result<RunControlReceipt, DomainError>>;
    /// Another member's harness changed the run (a pushed `watch`) or its
    /// work (a `wake`): this process's run watch reads the board now
    /// rather than at its refresh (#2390). No board call, no model turn,
    /// no prompt, and the wake cursor is left as it is.
    fn nudge_watch(&self);
    /// The run as its coordinator reads it at an idle boundary (#2467):
    /// `None` unless this process was launched as its run's coordinator.
    fn coordinator_board(&self) -> CoordinatorBoardFuture<'_>;
}

/// What a coordinator reads when one of its swarm workers ends a turn
/// (#2471): `None` unless this member is its run's coordinator.
pub trait WorkerBoardRead: Send + Sync {
    fn worker_board(&self) -> PortFuture<'_, Result<Option<WorkerBoard>, DomainError>>;
}

/// [`SwarmRunControl::coordinator_board`]'s answer.
pub type CoordinatorBoardFuture<'a> = PortFuture<'a, Result<Option<CoordinatorBoard>, DomainError>>;
