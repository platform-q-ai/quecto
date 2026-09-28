//! The member's session (#2285, #2287): its statistics, and the commands,
//! answers and views of the session use case
//! ([`crate::application::external_agent::use_cases::DriveExternalAgentSession`]).

use std::time::Duration;

use super::{
    ExecutionState, ExternalAgentExit, ExternalAgentInputError, ExternalAgentLaunchError,
    ExternalAgentLaunchSpec, ProjectionStep,
};
use crate::domain::external_agent::stream::TokenCounts;

/// The member's session statistics.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionTotals {
    pub session_key: Option<String>,
    pub model: Option<String>,
    pub user_messages: usize,
    pub assistant_messages: usize,
    pub tool_calls: usize,
    pub tool_results: usize,
    /// Every process's cumulative tokens (`modelUsage`), summed.
    pub tokens: TokenCounts,
    /// The session's cost at list price: the sum of every turn's charge,
    /// micro-USD.
    pub cost_micro_usd: u64,
    pub turns: usize,
    /// Every permission denial reported, including those the bounded
    /// audit no longer holds.
    pub guardrail_denials: usize,
    /// Every rate-limit event that warranted a warning.
    pub admission_warnings: usize,
    /// Every event of a type the vocabulary does not know.
    pub unknown_events: usize,
    /// Every line of the stream the adapter skipped unread (#2286).
    pub skipped_lines: usize,
}

/// The most follow-ups a busy member holds.
pub const FOLLOW_UP_QUEUE_CAPACITY: usize = 16;

/// How long the stream may stay quiet after a skipped line of a running
/// turn before the turn is given up as lost: the skipped line may have been
/// its `result`, which would never come again.
pub const SKIPPED_LINE_GRACE: Duration = Duration::from_secs(60);

/// What a member session is started with.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalAgentSessionSettings {
    pub launch: ExternalAgentLaunchSpec,
    /// See [`SKIPPED_LINE_GRACE`].
    pub skipped_line_grace: Duration,
}

/// How a prompt is delivered while a turn runs: quecto's
/// `streamingBehavior` (`protocol_commands.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingBehavior {
    /// Write it at once: it folds into the running turn.
    Steer,
    /// Hold it until the running turn ends, then start a turn with it.
    FollowUp,
}

/// What the session did with a prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptAccepted {
    /// It started turn `turn`.
    Started { turn: u64 },
    /// It was written at once, into the running turn `turn`.
    Steered { turn: u64 },
    /// It waits for the running turn to end, `position`th in line (from 1).
    Queued { position: usize },
}

/// Why the session refused a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionRefusal {
    /// The session has not started its agent.
    NotStarted,
    /// It has already started one.
    AlreadyStarted,
    /// A turn runs and the prompt named no [`StreamingBehavior`].
    Busy,
    /// [`FOLLOW_UP_QUEUE_CAPACITY`] follow-ups already wait.
    QueueFull,
    /// The member has ended: aborted, or its agent exited.
    Ended,
    /// The agent could not be started.
    Launch(ExternalAgentLaunchError),
    /// The prompt could not be written to the agent.
    Input(ExternalAgentInputError),
}

impl SessionRefusal {
    /// The refusal's kind, for telemetry: no detail.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotStarted => "not_started",
            Self::AlreadyStarted => "already_started",
            Self::Busy => "busy",
            Self::QueueFull => "queue_full",
            Self::Ended => "ended",
            Self::Launch(_) => "launch",
            Self::Input(_) => "input",
        }
    }
}

impl std::fmt::Display for SessionRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotStarted => write!(f, "the claude-code member has not started"),
            Self::AlreadyStarted => write!(f, "the claude-code member has already started"),
            // quecto's own busy refusal, word for word.
            Self::Busy => write!(f, "agent is running; provide streamingBehavior"),
            Self::QueueFull => write!(
                f,
                "the follow-up queue is full ({FOLLOW_UP_QUEUE_CAPACITY}); wait for the running turn to end"
            ),
            Self::Ended => write!(f, "the claude-code member has ended"),
            Self::Launch(error) => write!(f, "{error}"),
            Self::Input(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for SessionRefusal {}

/// Where the session is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SessionPhase {
    #[default]
    NotStarted,
    /// No turn runs: a prompt starts one.
    Idle,
    /// Turn `turn` runs.
    Busy { turn: u64 },
    /// The member has ended; nothing more is accepted.
    Ended,
}

/// What `abort` ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AbortOutcome {
    /// The turn that was running, if one was.
    pub turn: Option<u64>,
    /// The follow-ups that were waiting and are dropped.
    pub dropped_follow_ups: usize,
}

/// What one read of the agent's stream changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStep {
    /// An event was folded; a turn it ended starts the next follow-up.
    Folded(ProjectionStep),
    /// Turn `turn` was given up: a line of it was skipped and the stream
    /// then stayed quiet for the grace period.
    TurnLost { turn: u64 },
    /// The agent's output ended, cutting `turn` short if one ran: the
    /// member has ended.
    Ended {
        turn: Option<u64>,
        exit: ExternalAgentExit,
    },
}

/// The session as `get_state` sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionView {
    pub phase: SessionPhase,
    pub execution: ExecutionState,
    pub queued_follow_ups: usize,
    pub totals: SessionTotals,
}
