//! The words of one termination (#1935): the caller's protocol outcome,
//! the bounds of the fallback, and how it ended. Declared beside
//! [`super::OwnedChildSupervisor`] and re-exported from it.

use std::time::Duration;

use super::ChildExit;

/// Result of the caller's protocol attempt, in the supervisor's words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolOutcome {
    /// The child acknowledged the shutdown; it is expected to exit by itself.
    Acknowledged,
    /// The child could not be reached, refused, or the attempt timed out.
    Negative(String),
}

/// Bounded waits of one termination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminationBudget {
    /// How long an acknowledged child gets to exit before the fallback.
    pub exit_after_ack: Duration,
    /// How long after TERM before KILL.
    pub term_grace: Duration,
    /// How long after KILL before giving up on observing the exit.
    pub kill_grace: Duration,
}

impl TerminationBudget {
    pub const DEFAULT: Self = Self {
        exit_after_ack: Duration::from_secs(10),
        term_grace: Duration::from_secs(2),
        kill_grace: Duration::from_secs(2),
    };
}

impl Default for TerminationBudget {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminationOutcome {
    /// No unreaped handle is retained for this id: nothing was signalled.
    NoRetainedHandle,
    /// The child had already exited before any step was needed.
    AlreadyExited(ChildExit),
    /// The child exited on its own after acknowledging the protocol.
    ExitedAfterProtocol(ChildExit),
    /// The protocol outcome was negative (or the exit deadline passed) and
    /// TERM produced the exit.
    ExitedAfterTerm { negative: String, exit: ChildExit },
    /// TERM did not suffice within its grace; KILL produced the exit.
    ExitedAfterKill { negative: String, exit: ChildExit },
    /// Even KILL did not yield an observed exit within the budget.
    StillRunning { negative: String },
}
