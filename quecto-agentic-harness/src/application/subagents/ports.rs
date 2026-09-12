//! Capability-local effect ports for subagent teardown.
//!
//! Each port is owned by a use case in this capability; infrastructure
//! implements them and composition wires concrete instances. Signatures name
//! only domain and application types: no socket, JSON, process or file
//! vocabulary crosses this boundary.
use std::fmt;
use std::future::Future;
use std::pin::Pin;

use crate::domain::subagent_teardown::{
    DelegatedAgentIdentity, HarnessLifecycleState, LineageSnapshot, RoutingDepth, ShutdownReason,
};

pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Failure to deliver a control command to a direct child, in this
/// capability's own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildRoutingError {
    /// The child is not (or no longer) a direct child this harness controls.
    NotADirectChild,
    /// The child could not be reached or did not acknowledge.
    Unreachable(String),
}

impl fmt::Display for ChildRoutingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotADirectChild => f.write_str("not a direct child of this harness"),
            Self::Unreachable(detail) => write!(f, "unreachable: {detail}"),
        }
    }
}

/// Routes control across exactly one edge: this harness → a direct child.
pub trait DirectChildRouting: Send + Sync {
    /// Ask a direct child to shut itself (and its subtree) down. Resolves once
    /// the child has acknowledged the request, not once it has exited.
    ///
    /// Must be idempotent per child: the use case records each child as it
    /// completes and never re-sends to a recorded one, but a run interrupted
    /// between the child's acknowledgement and that record may ask once
    /// more, and the child (already shutting down) must treat that as a
    /// join, not a second teardown.
    fn shutdown_child<'a>(
        &'a self,
        child: &'a DelegatedAgentIdentity,
        reason: ShutdownReason,
    ) -> PortFuture<'a, Result<(), ChildRoutingError>>;

    /// Forward a selected termination one hop with the remaining budget.
    fn forward_termination<'a>(
        &'a self,
        via: &'a DelegatedAgentIdentity,
        target: &'a DelegatedAgentIdentity,
        remaining_depth: RoutingDepth,
    ) -> PortFuture<'a, Result<(), ChildRoutingError>>;
}

/// This harness's lifecycle state and its known delegation lineage.
pub trait SubagentLifecycleRepository: Send + Sync {
    fn lifecycle(&self) -> HarnessLifecycleState;
    /// Store a state the domain has already validated as a legal transition.
    fn set_lifecycle(&self, state: HarnessLifecycleState);
    fn lineage(&self) -> LineageSnapshot;
}

/// Cancels whatever turn is in flight so shutdown never waits on a provider.
pub trait TurnCancellation: Send + Sync {
    /// Returns `true` when a turn was actually interrupted.
    fn cancel_in_flight_turn(&self) -> PortFuture<'_, bool>;
}

/// Persists the session as part of shutdown, for the recorded reason.
pub trait ShutdownSessionPersistence: Send + Sync {
    fn persist_for_shutdown(&self, reason: ShutdownReason) -> PortFuture<'_, Result<(), String>>;
}

/// Milliseconds on a monotonic scale chosen by the adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ShutdownInstant(pub u64);

pub trait ShutdownClock: Send + Sync {
    fn now(&self) -> ShutdownInstant;
}

/// Why composition is being told the harness may exit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitReadiness {
    /// The common teardown ran to completion for this reason.
    Completed(ShutdownReason),
    /// The teardown could not be driven to completion after its ACK was on
    /// the wire; composition must still exit (falling back to process exit)
    /// so the parent never sees an ACK followed by nothing.
    Abandoned {
        reason: ShutdownReason,
        detail: String,
    },
}

impl ExitReadiness {
    pub const fn reason(&self) -> ShutdownReason {
        match self {
            Self::Completed(reason) | Self::Abandoned { reason, .. } => *reason,
        }
    }
}

/// Tells composition the harness may now exit; composition owns the actual
/// process exit and the order in which runtimes stop.
pub trait CompositionExitReadiness: Send + Sync {
    fn signal_exit_ready(&self, readiness: ExitReadiness) -> PortFuture<'_, ()>;
}

/// The executed teardown, detached from whichever caller admitted it.
pub type ShutdownRun = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

/// Runs the teardown independently of the caller's future, so a dropped
/// connection task can never abandon a shutdown whose ACK is already on the
/// wire. Composition supplies the runtime; the transaction owns the run.
pub trait ShutdownRunSpawner: Send + Sync {
    fn spawn_shutdown_run(&self, run: ShutdownRun);
}
