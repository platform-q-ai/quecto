//! Fold an external agent's event stream into the sub-agent protocol's
//! values (#2285), following spike #2264's mapping table.
//!
//! The projection answers in this capability's own DTOs; the interface
//! layer renders them as `AgentEvent` / `SessionState`, so the wire
//! protocol stays out of the application layer.
//!
//! - Messages get ordinals in arrival order. One API message arrives as
//!   several assistant events sharing a `message_id`; they form one
//!   message whose ordinal is assigned on its first block.
//! - User turns are recorded by the session ([`Projector::record_user_turn`]):
//!   without `--replay-user-messages` the stream never echoes them.
//! - A `result` ends a turn: it is classified ([`TurnEnd::classify`]), its
//!   usage recorded ([`UsageLedger`]), its text becomes the final report,
//!   and its permission denials the guardrail audit.
//! - `system/init` repeats every turn and never resets what was folded.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::domain::external_agent::stream::{
    AssistantContent, ExternalAgentEvent, InitEvent, PermissionDenial, RateLimitInfo, ResultEvent,
    TaskNotification, TaskStarted, TokenCounts, ToolResultEvent,
};
use crate::domain::external_agent::turn::{TurnEnd, TurnUsage};
use crate::domain::message::StopReason;

/// A final report is delivered in pages of this many bytes, the same
/// budget as a quecto child's final report (#2114).
pub const FINAL_REPORT_PAGE_BYTES: usize = 64 * 1024;

/// What the member is doing, as `get_state` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExecutionState {
    #[default]
    Idle,
    Thinking,
    Streaming,
    RunningTool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
    Tool,
}

/// One tool call of an assistant message.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectedToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

/// One message of the member's conversation.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectedMessage {
    pub ordinal: u64,
    pub role: MessageRole,
    pub content: String,
    pub tool_calls: Vec<ProjectedToolCall>,
    /// Thinking text; `None` while the CLI redacts it.
    pub thinking: Option<String>,
    /// The API message id an assistant message was grouped by.
    pub api_message_id: Option<String>,
    /// The tool call a tool message answers.
    pub tool_call_id: Option<String>,
    pub is_error: bool,
    /// A tool message whose call a permission rule (a hook) refused.
    pub permission_denied: bool,
}

/// The member's final report: the last turn's `result` text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FinalReport {
    pub content: String,
    /// The last assistant message with text when the report arrived.
    pub message_ordinal: Option<u64>,
}

impl FinalReport {
    pub fn full_length_bytes(&self) -> usize {
        self.content.len()
    }

    /// The report split into pages of at most [`FINAL_REPORT_PAGE_BYTES`],
    /// each cut on a character boundary. An empty report is one empty page.
    pub fn pages(&self) -> Vec<&str> {
        // RED (#2285): not implemented yet.
        vec![self.content.as_str()]
    }
}

/// How one turn ended and what it used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnOutcome {
    pub end: TurnEnd,
    pub stop_reason: Option<StopReason>,
    pub usage: TurnUsage,
    pub num_turns: Option<u32>,
    pub duration_ms: Option<u64>,
}

/// A tool call a permission rule refused, from `result.permission_denials`.
#[derive(Debug, Clone, PartialEq)]
pub struct GuardrailDenial {
    pub tool_name: String,
    pub tool_use_id: String,
    pub tool_input: Value,
    /// The turn (0-based) whose result reported it.
    pub turn: usize,
}

/// A Bash command the CLI tracks as a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundJob {
    pub task_id: String,
    pub tool_use_id: Option<String>,
    pub description: Option<String>,
    pub is_backgrounded: bool,
    /// The last status a notification gave; `None` while it runs.
    pub status: Option<String>,
}

/// The member's session statistics.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SessionTotals {
    pub session_key: Option<String>,
    pub model: Option<String>,
    pub user_messages: usize,
    pub assistant_messages: usize,
    pub tool_calls: usize,
    pub tool_results: usize,
    /// Cumulative tokens (`modelUsage`).
    pub tokens: TokenCounts,
    /// Cumulative cost at list price, micro-USD.
    pub cost_micro_usd: u64,
    pub turns: usize,
}

/// Folds [`ExternalAgentEvent`]s into the member's protocol values.
#[derive(Debug, Default)]
pub struct Projector {
    state: ExecutionState,
    transitions: Vec<ExecutionState>,
    messages: Vec<ProjectedMessage>,
    init: Option<InitEvent>,
    turns: Vec<TurnOutcome>,
    report: Option<FinalReport>,
    rate_limit: Option<RateLimitInfo>,
    admission_warnings: Vec<RateLimitInfo>,
    guardrail_audit: Vec<GuardrailDenial>,
    background_jobs: BTreeMap<String, BackgroundJob>,
    unknown_events: usize,
}

impl Projector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a user turn the session wrote to the agent; returns its
    /// ordinal.
    pub fn record_user_turn(&mut self, text: &str) -> u64 {
        // RED (#2285): not implemented yet.
        let _ = text;
        0
    }

    /// Fold one event. Returns the turn's outcome when `event` ends one.
    pub fn apply(&mut self, event: &ExternalAgentEvent) -> Option<TurnOutcome> {
        // RED (#2285): not implemented yet.
        let _ = event;
        None
    }

    pub fn state(&self) -> ExecutionState {
        self.state
    }

    /// Every state entered, in order (each differs from the one before).
    pub fn transitions(&self) -> &[ExecutionState] {
        &self.transitions
    }

    pub fn messages(&self) -> &[ProjectedMessage] {
        &self.messages
    }

    /// The latest `system/init`.
    pub fn init(&self) -> Option<&InitEvent> {
        self.init.as_ref()
    }

    pub fn turns(&self) -> &[TurnOutcome] {
        &self.turns
    }

    pub fn report(&self) -> Option<&FinalReport> {
        self.report.as_ref()
    }

    /// The latest rate-limit standing.
    pub fn rate_limit(&self) -> Option<&RateLimitInfo> {
        self.rate_limit.as_ref()
    }

    /// Every rate-limit event whose status was not clear.
    pub fn admission_warnings(&self) -> &[RateLimitInfo] {
        &self.admission_warnings
    }

    pub fn guardrail_audit(&self) -> &[GuardrailDenial] {
        &self.guardrail_audit
    }

    pub fn background_jobs(&self) -> impl Iterator<Item = &BackgroundJob> {
        self.background_jobs.values()
    }

    pub fn unknown_events(&self) -> usize {
        self.unknown_events
    }

    pub fn session_totals(&self) -> SessionTotals {
        // RED (#2285): not implemented yet.
        SessionTotals::default()
    }
}

#[cfg(test)]
#[path = "projection_tests.rs"]
mod tests;
