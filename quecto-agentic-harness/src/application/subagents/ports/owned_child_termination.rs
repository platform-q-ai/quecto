//! Selected termination of a directly owned child (#1936): how it ended,
//! the protocol attempt, the conclusion budget and the owned-handle fallback.

use super::PortFuture;
use super::child_routing::{ChildRoutingError, DownstreamRejection};
use crate::domain::agents::services::subagent_teardown::DelegatedAgentIdentity;
use std::fmt;

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

impl ProtocolAttempt {
    /// How a `shutdown` answer reads as an attempt. An acknowledgement, or
    /// a child that answered that it is already ending (kind
    /// `already_exited`: its harness already terminated, its exit follows),
    /// is an acknowledged attempt — the exit is awaited within the budget
    /// and no signal is authorised before it. Every other answer —
    /// unreachable, a refusal of another kind, a malformed acknowledgement
    /// — is negative.
    pub fn from_shutdown_answer(answer: Result<(), ChildRoutingError>) -> Self {
        match answer {
            Ok(()) | Err(ChildRoutingError::Downstream(DownstreamRejection::AlreadyExited)) => {
                Self::Acknowledged
            }
            Err(error) => Self::Negative(error.to_string()),
        }
    }
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
