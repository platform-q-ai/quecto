//! What the member session records of each of its decisions and effects
//! (#2287): ids, kinds, sizes and durations only, never a prompt, a
//! transcript or a credential. The adapter logs each under the
//! `quecto::external_agent` target; E2-S11 (#2304) adds them to the
//! member's event log.

use super::PromptAccepted;
use crate::domain::external_agent::telemetry::{
    ExternalAgentStreamDiagnostic, ExternalAgentTool, ExternalAgentTurn,
};

/// The most of a tool's name a [`SessionRecord::ToolCalled`] keeps, in
/// bytes; it keeps only ASCII letters, digits and `_ - . :`, each other
/// character becoming `?`.
pub const TOOL_NAME_RECORD_BYTES: usize =
    crate::domain::external_agent::telemetry::RECORDED_NAME_BYTES;

/// One decision or effect of a member session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionRecord {
    /// The agent process started.
    Started,
    /// It could not be started: the launch error's kind.
    StartRefused { kind: &'static str },
    /// A prompt of `bytes` bytes was accepted.
    PromptAccepted {
        accepted: PromptAccepted,
        bytes: usize,
    },
    /// A prompt of `bytes` bytes was refused (a [`super::SessionRefusal`]
    /// kind).
    PromptRefused { refusal: &'static str, bytes: usize },
    /// Turn `turn` called tool `tool` (its name only, bounded by
    /// [`TOOL_NAME_RECORD_BYTES`]).
    ToolCalled { turn: Option<u64>, tool: String },
    /// A line of `bytes` bytes was skipped during `turn`.
    LineSkipped { turn: Option<u64>, bytes: usize },
    /// Turn `turn` ended: `completed`, `failed`, `aborted` (the agent's
    /// own abort, or an interrupt's) or `exited`, with what its result
    /// said it took.
    TurnEnded {
        turn: u64,
        outcome: &'static str,
        duration_ms: Option<u64>,
        cost_micro_usd: u64,
    },
    /// A result came for turn `turn` while `owed` user turns written into
    /// it are still unanswered: the turn goes on.
    TurnContinued { turn: u64, owed: usize },
    /// A result naming no user turn came while turn `turn` ran, after
    /// claude had named them before: `ended` when it was taken for the
    /// turn's end (an error result: a session-scoped failure, a zeroed or
    /// delivery-failure result), not when it was a turn of claude's own (a
    /// success that consumed no user turn of the member's).
    ResultWithoutIds { turn: u64, ended: bool },
    /// Turn `turn` was interrupted: `abort`, or `lost` (a skipped line,
    /// then silence).
    Interrupted { turn: u64, cause: &'static str },
    /// Interrupted turn `turn` did not answer in time, or the interrupt
    /// could not be written: the member was ended, dropping
    /// `dropped_follow_ups` queued follow-ups.
    Abandoned {
        turn: u64,
        dropped_follow_ups: usize,
    },
    /// A queued follow-up started turn `turn`.
    FollowUpStarted { turn: u64, bytes: usize },
    /// The follow-up of `bytes` bytes that was to start turn `turn` could
    /// not be written (a [`super::SessionRefusal`] kind).
    FollowUpFailed {
        turn: u64,
        bytes: usize,
        refusal: &'static str,
    },
    /// The member was aborted, ending `turn` and dropping the follow-ups.
    Aborted {
        turn: Option<u64>,
        dropped_follow_ups: usize,
    },
    /// The member was closed, ending `turn` and dropping the follow-ups.
    Closed {
        turn: Option<u64>,
        dropped_follow_ups: usize,
    },
    /// The agent's output ended; `clean` when it exited with status 0,
    /// `wall_ms` after its start (#2304).
    Ended {
        clean: bool,
        exit_code: Option<i32>,
        signal: Option<i32>,
        wall_ms: Option<u64>,
    },
    /// A turn ended, as the event log keeps it (#2304): recorded just
    /// before its [`Self::TurnEnded`].
    TurnReported(Box<ExternalAgentTurn>),
    /// A tool call was answered, or its turn ended first (#2304).
    ToolFinished(Box<ExternalAgentTool>),
    /// A process's first `system/init` (#2304).
    Initialized {
        cli_version: Option<String>,
        claude_session_id: Option<String>,
        model: Option<String>,
    },
    /// The stream said something the vocabulary could not read (#2304).
    StreamDiagnostic(ExternalAgentStreamDiagnostic),
}

impl SessionRecord {
    /// The record's kind: its variant, in snake case.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::StartRefused { .. } => "start_refused",
            Self::PromptAccepted { .. } => "prompt_accepted",
            Self::PromptRefused { .. } => "prompt_refused",
            Self::ToolCalled { .. } => "tool_called",
            Self::LineSkipped { .. } => "line_skipped",
            Self::TurnEnded { .. } => "turn_ended",
            Self::TurnContinued { .. } => "turn_continued",
            Self::ResultWithoutIds { .. } => "result_without_ids",
            Self::Interrupted { .. } => "interrupted",
            Self::Abandoned { .. } => "abandoned",
            Self::FollowUpStarted { .. } => "follow_up_started",
            Self::FollowUpFailed { .. } => "follow_up_failed",
            Self::Closed { .. } => "closed",
            Self::Aborted { .. } => "aborted",
            Self::Ended { .. } => "ended",
            Self::TurnReported(_) => "turn_reported",
            Self::ToolFinished(_) => "tool_finished",
            Self::Initialized { .. } => "initialized",
            Self::StreamDiagnostic(_) => "stream_diagnostic",
        }
    }
}
