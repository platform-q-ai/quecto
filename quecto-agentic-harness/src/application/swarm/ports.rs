//! Capability-local ports of the swarm lifecycle capability (#1940, #1960).
//!
//! Wire names, positional arguments and persistence schemas are adapter
//! details; every signature here names only domain types.
use std::future::Future;
use std::pin::Pin;

use crate::domain::error::DomainError;
use crate::domain::swarm::{
    Member, MemberExit, ProcessIdentity, RunControlAction, RunControlReceipt, RunStatus, Snapshot,
};

pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

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
    /// Cancel this member's detached execution registry independently of turn abort.
    fn cancel_local_executions(&self);
    /// Cancel current jobs while retaining admission for a later resume.
    fn suspend_local_executions(&self, snapshot: &Snapshot);
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
    ) -> PortFuture<'a, Result<(), DomainError>>;
    fn observed_outcome(&self, snapshot: &Snapshot, clock: &dyn Clock) -> RunStatus;
}

/// Supervisor operations remain available without model execution or turn-queue admission.
pub trait SwarmRunControl: Send + Sync {
    fn apply(
        &self,
        action: RunControlAction,
    ) -> PortFuture<'_, Result<RunControlReceipt, DomainError>>;
}
