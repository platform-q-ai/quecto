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
    /// SIGTERM/SIGINT delivered by the operating system.
    TerminationSignal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareShutdownRequest {
    pub reason: ShutdownReason,
    pub trigger: ShutdownTrigger,
}

/// Opaque proof that a shutdown was admitted. Only the transaction that
/// minted it can read it; callers hand it back to execute or release.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ShutdownToken(u64);

impl ShutdownToken {
    pub(super) const fn mint(nonce: u64) -> Self {
        Self(nonce)
    }

    /// Presenter tests need a token to shape an ACK; it never reaches a wire.
    #[cfg(test)]
    pub(crate) const fn for_presenter_tests() -> Self {
        Self(0)
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

/// What releasing an admission did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseOutcome {
    /// The last participant left: the freeze was lifted and nothing ran.
    Released,
    /// Other triggers still hold the admission; it stays frozen.
    StillHeld,
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
    /// The token does not belong to the current admission.
    UnknownToken,
    /// The harness already terminated: nothing further can be admitted.
    AlreadyTerminated,
    /// The lifecycle repository refused a transition it must never refuse
    /// for an admitted shutdown.
    LifecycleViolation(String),
    /// The caller running the teardown was cancelled before it completed;
    /// the admission is intact and `Execute` may be called again.
    ExecutionInterrupted,
}

impl fmt::Display for HarnessShutdownError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPrepared => f.write_str("no shutdown has been prepared"),
            Self::UnknownToken => f.write_str("shutdown token is not the admitted one"),
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
