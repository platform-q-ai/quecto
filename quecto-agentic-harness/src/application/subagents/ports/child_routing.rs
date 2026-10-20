//! Control across exactly one edge, this harness to a direct child: the
//! refusals a hop relays back, and how a selected termination ended (the
//! result a hop relays).

use super::PortFuture;
use crate::domain::agents::services::subagent_teardown::DelegatedAgentIdentity;
use crate::domain::agents::services::subagent_teardown::RoutingDepth;
use crate::domain::agents::services::subagent_teardown::ShutdownReason;
use std::fmt;

/// A refusal a harness further down the route answered with, relayed
/// distinctly so the root presents it truthfully instead of folding every
/// downstream answer into "unreachable".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownstreamRejection {
    /// The receiver's lineage does not list the target.
    UnknownTarget,
    /// The receiver lists the target at another launch generation.
    StaleGeneration,
    /// The receiver already observed the target's end.
    AlreadyExited,
    /// The receiver's routing policy refused for another reason.
    Rejected(String),
    /// The receiver is frozen or terminated.
    NotAccepting,
    /// The receiver could not reach its own next edge.
    Unreachable(String),
    /// The receiver dispatched effects to the target but did not observe
    /// its end within its bound.
    Failed(String),
}

impl DownstreamRejection {
    /// Closed vocabulary of rejection kinds as they cross a hop.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::UnknownTarget => "unknown_target",
            Self::StaleGeneration => "stale_generation",
            Self::AlreadyExited => "already_exited",
            Self::Rejected(_) => "rejected",
            Self::NotAccepting => "not_accepting",
            Self::Unreachable(_) => "unreachable",
            Self::Failed(_) => "failed",
        }
    }

    /// The rejection a kind names; an unknown kind is a plain rejection
    /// carrying the detail.
    pub fn from_kind(kind: &str, detail: &str) -> Self {
        match kind {
            "unknown_target" => Self::UnknownTarget,
            "stale_generation" => Self::StaleGeneration,
            "already_exited" => Self::AlreadyExited,
            "not_accepting" => Self::NotAccepting,
            "unreachable" => Self::Unreachable(detail.to_owned()),
            "failed" => Self::Failed(detail.to_owned()),
            _ => Self::Rejected(detail.to_owned()),
        }
    }

    /// Whether the downstream owner dispatched effects toward the target.
    pub const fn effects_dispatched(&self) -> bool {
        matches!(self, Self::Failed(_))
    }
}

impl fmt::Display for DownstreamRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTarget => f.write_str("target unknown downstream"),
            Self::StaleGeneration => f.write_str("target generation is stale downstream"),
            Self::AlreadyExited => f.write_str("target already exited downstream"),
            Self::Rejected(detail) => write!(f, "rejected downstream: {detail}"),
            Self::NotAccepting => f.write_str("downstream harness is not accepting"),
            Self::Unreachable(detail) => write!(f, "unreachable downstream: {detail}"),
            Self::Failed(detail) => write!(f, "termination failed downstream: {detail}"),
        }
    }
}

/// Failure to deliver a control command to a direct child, in this
/// capability's own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChildRoutingError {
    /// The child is not (or no longer) a direct child this harness controls.
    NotADirectChild,
    /// The child could not be reached or did not acknowledge.
    Unreachable(String),
    /// The child was reached and answered with a refusal of its own.
    Downstream(DownstreamRejection),
}

impl fmt::Display for ChildRoutingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotADirectChild => f.write_str("not a direct child of this harness"),
            Self::Unreachable(detail) => write!(f, "unreachable: {detail}"),
            Self::Downstream(rejection) => write!(f, "{rejection}"),
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

    /// Forward a selected termination one hop with the remaining budget and
    /// wait, within a bound that grows with the remaining depth, for the
    /// downstream answer: the result the target's owner relayed (`None`
    /// when the hop reported only that it routed the edge), or its refusal.
    fn forward_termination<'a>(
        &'a self,
        via: &'a DelegatedAgentIdentity,
        target: &'a DelegatedAgentIdentity,
        remaining_depth: RoutingDepth,
    ) -> PortFuture<'a, Result<Option<TerminationResult>, ChildRoutingError>>;
}

/// How the selected agent ended. `failed` is the error side
/// (`KillDelegatedAgentError::Failed`), never a success.
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
    /// The inverse of [`Self::as_str`]; `None` for any other word.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "graceful" => Some(Self::Graceful),
            "fallback" => Some(Self::Fallback),
            "already-exited" => Some(Self::AlreadyExited),
            _ => None,
        }
    }

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
