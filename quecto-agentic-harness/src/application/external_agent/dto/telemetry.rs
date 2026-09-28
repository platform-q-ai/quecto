//! What the member session records of each of its decisions and effects
//! (#2287): ids, kinds, sizes and durations only, never a prompt, a
//! transcript or a credential. The adapter logs each under the
//! `quecto::external_agent` target; E2-S11 (#2304) adds them to the
//! member's event log.

use super::PromptAccepted;

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
    /// Turn `turn` called tool `tool` (its name only).
    ToolCalled { turn: Option<u64>, tool: String },
    /// A line of `bytes` bytes was skipped during `turn`.
    LineSkipped { turn: Option<u64>, bytes: usize },
    /// Turn `turn` ended: `completed`, `failed`, `aborted` (the agent's
    /// own abort), `lost` or `exited`, with what its result said it took.
    TurnEnded {
        turn: u64,
        outcome: &'static str,
        duration_ms: Option<u64>,
        cost_micro_usd: u64,
    },
    /// A queued follow-up started turn `turn`.
    FollowUpStarted { turn: u64, bytes: usize },
    /// The member was aborted, ending `turn` and dropping the follow-ups.
    Aborted {
        turn: Option<u64>,
        dropped_follow_ups: usize,
    },
    /// The agent's output ended; `clean` when it exited with status 0.
    Ended { clean: bool },
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
            Self::FollowUpStarted { .. } => "follow_up_started",
            Self::Aborted { .. } => "aborted",
            Self::Ended { .. } => "ended",
        }
    }
}
