//! Capability-local effect ports for subagent teardown.
//!
//! Each port is owned by a use case in this capability; infrastructure
//! implements them and composition wires concrete instances. Signatures name
//! only domain and application types: no socket, JSON, process or file
//! vocabulary crosses this boundary.
use std::fmt;
use std::future::Future;
use std::pin::Pin;

use crate::domain::ids::AgentUuid;
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

// ─── Selected termination and lifecycle compensation (#1936) ─────────────────

/// The outcome of the protocol attempt this harness already made against a
/// directly owned child, handed to the owned-handle fallback so it can
/// decide whether a signal is authorised (#1935: only a negative outcome, or
/// an acknowledged child that never exits, authorises one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolAttempt {
    /// The child acknowledged the shutdown and is expected to exit itself.
    Acknowledged,
    /// The child could not be reached, refused, or its acknowledgement was
    /// malformed, mismatched or late.
    Negative(String),
}

/// How much patience a conclusion gets. A selected termination gives an
/// acknowledged child its full exit budget; a launch rollback of a child
/// that was never handed work only needs a short one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConclusionBudget {
    Standard,
    Rollback,
}

/// How the owned-handle fallback concluded a directly owned child.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminationConclusion {
    /// This harness retains no unreaped handle for the child (a script or
    /// container member, a merged descendant, a restored row): nothing
    /// beyond the protocol was possible and nothing was signalled.
    NoRetainedHandle,
    /// The child had already exited before any step was needed.
    AlreadyExited,
    /// The child exited on its own after the protocol acknowledgement.
    ExitedAfterProtocol,
    /// The protocol outcome was negative (or the exit deadline passed) and
    /// a fallback signal produced the exit.
    ExitedAfterFallback,
    /// Even the fallback did not yield an observed exit within its budget.
    StillRunning(String),
}

/// The only fallback owner (#1935): concludes a directly owned child after
/// the caller's protocol attempt, signalling the retained handle at most
/// once per kind and only when the attempt authorises it.
pub trait OwnedChildTermination: Send + Sync {
    fn conclude<'a>(
        &'a self,
        child: &'a DelegatedAgentIdentity,
        attempt: ProtocolAttempt,
        budget: ConclusionBudget,
    ) -> PortFuture<'a, TerminationConclusion>;
}

/// Why a delegated agent's row could not be resolved to one live identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolutionError {
    /// No row carries that uuid or live display label.
    Unknown,
    /// More than one live row answers to that display label.
    Ambiguous,
    /// The row is already exited or compensated.
    Exited,
    /// The row is not a delegated agent this harness can address: it was
    /// neither launched here nor reported with a launch generation.
    NotDelegated,
}

impl fmt::Display for ResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => f.write_str("not found in registry"),
            Self::Ambiguous => f.write_str("display label is ambiguous; use the uuid"),
            Self::Exited => f.write_str("already exited"),
            Self::NotDelegated => {
                f.write_str("not a delegated agent launched through this harness")
            }
        }
    }
}

/// Why a stopping claim was refused. Every refusal means no effect ran.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoppingClaimError {
    /// No row with that identity (uuid and generation) is known.
    Unknown,
    /// Another termination already claimed the row and is in flight.
    AlreadyStopping,
    /// The row is already terminal.
    Exited,
}

impl fmt::Display for StoppingClaimError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown => f.write_str("unknown delegated agent"),
            Self::AlreadyStopping => f.write_str("a termination is already in flight"),
            Self::Exited => f.write_str("already exited"),
        }
    }
}

/// What the harness observed about a child's end, in the words of the path
/// that observed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitObservation {
    /// The owned process was reaped (exit status observed).
    ProcessExited,
    /// The monitor connection reached EOF or was reset.
    ConnectionClosed,
    /// The monitor connection was never accepted.
    NeverReachable,
}

/// Why a row's terminal effects run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminationCause {
    /// The child ended on its own, observed this way.
    Exit(ExitObservation),
    /// An operator selected the agent for termination.
    SelectedTermination,
    /// A launch failed after registration; `owns_environment` says whether
    /// the launch created the environment it joined.
    LaunchRollback { owns_environment: bool },
}

/// Result of claiming a row's terminal effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalClaim {
    /// This caller owns the compensation and must run it.
    Claimed,
    /// Another path already claimed (or finished) it; join instead.
    AlreadyClaimed,
}

/// Outcome of waiting for a row's compensation to finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompensationObservation {
    /// The row's terminal effects have run.
    Compensated,
    /// The adapter's bound elapsed with the row still not compensated.
    TimedOut,
    /// No row with that identity exists (it was never registered, or was
    /// uncommitted).
    Unknown,
}

/// The launching harness's registry of delegated agents: identity
/// resolution, the per-row teardown claims that make every termination path
/// converge exactly once, and observation of a row's compensation.
pub trait DelegatedAgentRegistry: Send + Sync {
    /// Resolve an operator reference (uuid or live display label) to the
    /// one live delegated agent it names.
    fn resolve(&self, reference: &str) -> Result<DelegatedAgentIdentity, ResolutionError>;
    /// Mark the row stopping before any effect runs. Refused, with no
    /// effect, when the row is unknown, already stopping or terminal.
    fn claim_stopping(
        &self,
        target: &DelegatedAgentIdentity,
        cause: TerminationCause,
    ) -> Result<(), StoppingClaimError>;
    /// Lift a stopping claim whose effects were refused or failed, so a
    /// later trigger may try again. Idempotent.
    fn release_stopping(&self, target: &DelegatedAgentIdentity);
    /// Claim the row's terminal effects. Exactly one caller ever gets
    /// `Claimed` for a row.
    fn claim_terminal(&self, target: &DelegatedAgentIdentity) -> TerminalClaim;
    /// Whether this harness still retains the child's unreaped process, in
    /// which case its reaper — not a connection edge — observes the exit.
    fn holds_process(&self, target: &DelegatedAgentIdentity) -> bool;
    /// Whether the row's terminal effects were already claimed (or ran):
    /// another path observed its exit. Never blocks, never claims.
    fn terminal_claimed(&self, target: &DelegatedAgentIdentity) -> bool;
    /// Wait, within the adapter's bound, for the row's compensation.
    fn await_compensated<'a>(
        &'a self,
        target: &'a DelegatedAgentIdentity,
    ) -> PortFuture<'a, CompensationObservation>;
}

/// The rows a compensation removed from live membership: the target and
/// every descendant reported beneath it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compensated {
    pub removed: Vec<AgentUuid>,
}

/// Runs a claimed row's terminal effects exactly once: environment cleanup
/// and membership removal, monitor and bridge teardown, removal of the row
/// and its reported subtree from live membership, one survivor broadcast
/// and one notification. Never signals a process.
pub trait TeardownCompensation: Send + Sync {
    fn compensate<'a>(
        &'a self,
        target: &'a DelegatedAgentIdentity,
        cause: TerminationCause,
    ) -> PortFuture<'a, Compensated>;
}
