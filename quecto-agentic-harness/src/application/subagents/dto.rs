//! Boundary request/result models for the subagent teardown capability.
use std::fmt;

use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::{
    DelegatedAgentIdentity, RoutingDepth, ShutdownReason, TerminationRouteError,
};

/// Independent inbound events that all converge on one common shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShutdownTrigger {
    /// A `shutdown` protocol command on an authorized connection.
    ProtocolCommand,
    /// The authenticated launch-bound parent connection closed.
    ParentConnectionClosed,
    /// No parent bound its control connection within the bind deadline.
    ParentNeverBound,
    /// SIGTERM/SIGINT delivered by the operating system.
    TerminationSignal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareShutdownRequest {
    pub reason: ShutdownReason,
    pub trigger: ShutdownTrigger,
}

/// Opaque proof that one caller holds the admitted shutdown. Every
/// participant gets its own token (one admission, one holder each), so a
/// holder's release can never be mistaken for another's. Only the
/// transaction that minted it can read it.
///
/// A token is a plain value: it has no drop behaviour. A holder that loses
/// it (or `mem::forget`s it) without releasing or executing leaks the freeze
/// until another trigger executes; keeping the token reachable is the
/// caller's responsibility (the UDS controller guards its own).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ShutdownToken {
    admission: u64,
    holder: u64,
}

impl ShutdownToken {
    pub(super) const fn mint(admission: u64, holder: u64) -> Self {
        Self { admission, holder }
    }

    pub(super) const fn admission(&self) -> u64 {
        self.admission
    }

    pub(super) const fn holder(&self) -> u64 {
        self.holder
    }

    /// Presenter tests need a token to shape an ACK; it never reaches a wire.
    #[cfg(test)]
    pub(crate) const fn for_presenter_tests() -> Self {
        Self {
            admission: 0,
            holder: 0,
        }
    }
}

impl fmt::Debug for ShutdownToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ShutdownToken(..)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedShutdown {
    pub token: ShutdownToken,
    /// `true` when an earlier trigger already admitted the shutdown and this
    /// caller joined it instead of admitting a second one.
    pub joined: bool,
    /// Reason recorded by the admitting trigger; later joiners inherit it.
    pub reason: ShutdownReason,
}

/// What releasing one holder's admission did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseOutcome {
    /// The last holder left: the freeze was lifted and nothing ran.
    Released,
    /// Other holders still hold the admission; it stays frozen.
    StillHeld,
    /// This holder had already released; nothing changed.
    AlreadyReleased,
    /// Execution already began or finished; a release changes nothing.
    ExecutionUnderway,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistenceOutcome {
    Persisted,
    Failed(String),
}

/// Result of the executed common shutdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownOutcome {
    pub reason: ShutdownReason,
    /// Every trigger that admitted or joined this shutdown, first one first.
    pub triggers: Vec<ShutdownTrigger>,
    pub turn_cancelled: bool,
    pub children_shut_down: Vec<AgentUuid>,
    pub children_failed: Vec<(AgentUuid, String)>,
    pub persistence: PersistenceOutcome,
    pub exit_signalled: bool,
}

/// Terminal error vocabulary of the two-phase shutdown transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarnessShutdownError {
    /// Execute or release without any admitted shutdown.
    NotPrepared,
    /// The token does not belong to the current admission or names no
    /// holder of it.
    UnknownToken,
    /// The token's holder already released it; a released holder can neither
    /// execute nor release again.
    TokenReleased,
    /// The harness already terminated: nothing further can be admitted.
    AlreadyTerminated,
    /// The lifecycle repository refused a transition it must never refuse
    /// for an admitted shutdown.
    LifecycleViolation(String),
    /// The spawned teardown run was dropped before it completed (its runtime
    /// went away); the admission and its progress are intact and `Execute`
    /// resumes at the first incomplete step.
    ExecutionInterrupted,
}

impl fmt::Display for HarnessShutdownError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPrepared => f.write_str("no shutdown has been prepared"),
            Self::UnknownToken => f.write_str("shutdown token is not the admitted one"),
            Self::TokenReleased => f.write_str("shutdown token was already released"),
            Self::AlreadyTerminated => f.write_str("harness already terminated"),
            Self::LifecycleViolation(detail) => write!(f, "lifecycle violation: {detail}"),
            Self::ExecutionInterrupted => f.write_str("shutdown execution was interrupted"),
        }
    }
}

impl std::error::Error for HarnessShutdownError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminateDelegatedAgentRequest {
    pub target: DelegatedAgentIdentity,
    pub remaining_depth: RoutingDepth,
}

/// The single edge this harness acted on; it never shuts itself down here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminationRouted {
    /// The target was a direct child and self shutdown was invoked on it.
    ShutdownRequested { child: DelegatedAgentIdentity },
    /// The command was forwarded one hop; this harness stays alive.
    Forwarded {
        via: DelegatedAgentIdentity,
        remaining_depth: RoutingDepth,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminateDelegatedAgentError {
    /// Routing policy refused: no edge was touched.
    Rejected(TerminationRouteError),
    /// The routing port failed to reach the resolved direct child.
    ChildUnreachable { child: AgentUuid, detail: String },
    /// The receiving harness is itself frozen or terminated.
    NotAccepting,
}

impl fmt::Display for TerminateDelegatedAgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(error) => write!(f, "termination rejected: {error}"),
            Self::ChildUnreachable { child, detail } => {
                write!(f, "direct child {child} unreachable: {detail}")
            }
            Self::NotAccepting => f.write_str("harness is not accepting control commands"),
        }
    }
}

impl std::error::Error for TerminateDelegatedAgentError {}

// ─── Operator-selected termination (#1936, #1882) ────────────────────────────

/// An operator asks this harness to terminate one delegated agent, named
/// by uuid or live display label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KillDelegatedAgentRequest {
    pub reference: String,
}

/// How the selected agent ended. `failed` is the error side
/// ([`KillDelegatedAgentError::Failed`]), never a success.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminationResult {
    /// The child acknowledged the protocol and its exit was observed; no
    /// fallback signal was sent.
    Graceful,
    /// The protocol did not suffice and the directly owned handle's
    /// fallback produced the exit.
    Fallback,
    /// The child had already exited when the termination reached it.
    AlreadyExited,
}

impl TerminationResult {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Graceful => "graceful",
            Self::Fallback => "fallback",
            Self::AlreadyExited => "already-exited",
        }
    }
}

impl fmt::Display for TerminationResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KillDelegatedAgentOutcome {
    pub target: DelegatedAgentIdentity,
    pub result: TerminationResult,
    /// The target and every descendant removed with it, target first.
    pub removed: Vec<AgentUuid>,
}

/// Every error leaves the registry as it was: a refusal happens before any
/// effect, and a failed termination lifts its stopping claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KillDelegatedAgentError {
    /// The reference names no live delegated agent (unknown, ambiguous,
    /// exited, or not launched through this harness).
    Unresolved(super::ports::ResolutionError),
    /// Another termination of the same agent is already in flight.
    AlreadyStopping,
    /// Routing policy refused: stale generation, cycle, depth, or the
    /// lineage disappeared between resolution and routing.
    Rejected(TerminationRouteError),
    /// The receiving harness is itself frozen or terminated.
    NotAccepting,
    /// The route toward a nested target could not be delivered: the direct
    /// child it goes through did not accept the command. No fallback exists
    /// for a target this harness does not own.
    RouteUnreachable { via: AgentUuid, detail: String },
    /// The termination ran but the agent's end was not observed: the
    /// protocol failed with no retained handle to fall back on, the exit
    /// was not observed within the bound, or the fallback did not end it.
    Failed { detail: String },
}

impl fmt::Display for KillDelegatedAgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unresolved(error) => write!(f, "{error}"),
            Self::AlreadyStopping => f.write_str("a termination is already in flight"),
            Self::Rejected(error) => write!(f, "termination rejected: {error}"),
            Self::NotAccepting => f.write_str("harness is not accepting control commands"),
            Self::RouteUnreachable { via, detail } => {
                write!(f, "route via {via} unreachable: {detail}")
            }
            Self::Failed { detail } => write!(f, "termination failed: {detail}"),
        }
    }
}

impl std::error::Error for KillDelegatedAgentError {}

// ─── Owned child exit observation ────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObserveOwnedChildExitRequest {
    pub child: DelegatedAgentIdentity,
    pub observation: super::ports::ExitObservation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservedExit {
    /// This observation claimed and ran the row's terminal effects.
    Compensated { removed: Vec<AgentUuid> },
    /// Another path had already claimed them; this observation joined it.
    Joined(super::ports::CompensationObservation),
    /// A connection-level observation of a child whose process this harness
    /// still retains: the reaper observes the authoritative exit and runs
    /// the compensation, so nothing was removed while the process lived.
    DeferredToProcessExit,
}

// ─── Failed launch compensation ──────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompensateFailedLaunchRequest {
    pub child: DelegatedAgentIdentity,
    /// Whether this launch created the environment it joined, so its
    /// rollback discards the record rather than listing it as stopped.
    pub owns_environment: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedLaunchCompensated {
    pub conclusion: super::ports::TerminationConclusion,
    pub removed: Vec<AgentUuid>,
}
