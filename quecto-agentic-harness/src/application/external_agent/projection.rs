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

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

use crate::domain::external_agent::stream::{
    AssistantContent, ExternalAgentEvent, InitEvent, PermissionDenial, RateLimitInfo, ResultEvent,
    TaskNotification, TaskStarted, TokenCounts, ToolResultEvent,
};
use crate::domain::external_agent::turn::{TurnEnd, TurnUsage, UsageLedger};
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

impl ProjectedMessage {
    fn new(ordinal: u64, role: MessageRole) -> Self {
        Self {
            ordinal,
            role,
            content: String::new(),
            tool_calls: Vec::new(),
            thinking: None,
            api_message_id: None,
            tool_call_id: None,
            is_error: false,
            permission_denied: false,
        }
    }
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
        let mut pages = Vec::new();
        let mut rest = self.content.as_str();
        loop {
            let mut end = rest.len().min(FINAL_REPORT_PAGE_BYTES);
            while !rest.is_char_boundary(end) {
                end -= 1;
            }
            // A page always advances unless the rest is empty.
            debug_assert!(end > 0 || rest.is_empty());
            let (page, tail) = rest.split_at(end);
            pages.push(page);
            rest = tail;
            if rest.is_empty() {
                return pages;
            }
        }
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
    by_api_message: HashMap<String, usize>,
    init: Option<InitEvent>,
    ledger: UsageLedger,
    turns: Vec<TurnOutcome>,
    report: Option<FinalReport>,
    turn_assistant_error: Option<String>,
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
        let index = self.push(MessageRole::User);
        self.messages[index].content = text.to_string();
        self.messages[index].ordinal
    }

    /// Fold one event. Returns the turn's outcome when `event` ends one.
    pub fn apply(&mut self, event: &ExternalAgentEvent) -> Option<TurnOutcome> {
        match event {
            ExternalAgentEvent::Init(init) => {
                self.init = Some(init.clone());
                self.enter(ExecutionState::Thinking);
            }
            ExternalAgentEvent::ThinkingTokens { .. } => self.enter(ExecutionState::Thinking),
            ExternalAgentEvent::AssistantBlock { message_id, block } => {
                self.assistant_block(message_id, block)
            }
            ExternalAgentEvent::AssistantError { kind, .. } => {
                self.turn_assistant_error = Some(kind.clone());
            }
            ExternalAgentEvent::ToolResult(result) => self.tool_result(result),
            ExternalAgentEvent::UserText { text } => {
                self.record_user_turn(text);
            }
            ExternalAgentEvent::TaskStarted(task) => self.task_started(task),
            ExternalAgentEvent::TaskNotification(note) => self.task_notification(note),
            ExternalAgentEvent::BackgroundTasksChanged { tasks } => {
                for task in tasks {
                    self.job(&task.task_id).description = task.description.clone();
                }
            }
            ExternalAgentEvent::RateLimit(info) => self.record_rate_limit(info),
            ExternalAgentEvent::Result(result) => return Some(self.result(result)),
            ExternalAgentEvent::Unknown { .. } => self.unknown_events += 1,
        }
        None
    }

    fn push(&mut self, role: MessageRole) -> usize {
        let ordinal = self.messages.len() as u64;
        self.messages.push(ProjectedMessage::new(ordinal, role));
        self.messages.len() - 1
    }

    fn enter(&mut self, state: ExecutionState) {
        if self.state != state {
            self.state = state;
            self.transitions.push(state);
        }
    }

    fn assistant_block(&mut self, message_id: &str, block: &AssistantContent) {
        let index = match self.by_api_message.get(message_id) {
            Some(index) => *index,
            None => {
                let index = self.push(MessageRole::Assistant);
                self.messages[index].api_message_id = Some(message_id.to_string());
                self.by_api_message.insert(message_id.to_string(), index);
                index
            }
        };
        let message = &mut self.messages[index];
        assert_eq!(message.role, MessageRole::Assistant);
        match block {
            AssistantContent::Text(text) => {
                message.content.push_str(text);
                self.enter(ExecutionState::Streaming);
            }
            AssistantContent::ToolUse { id, name, input } => {
                message.tool_calls.push(ProjectedToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: input.clone(),
                });
                self.enter(ExecutionState::RunningTool);
            }
            AssistantContent::Thinking { text } => {
                if let Some(text) = text {
                    message
                        .thinking
                        .get_or_insert_with(String::new)
                        .push_str(text);
                }
                self.enter(ExecutionState::Thinking);
            }
        }
    }

    fn tool_result(&mut self, result: &ToolResultEvent) {
        let index = self.push(MessageRole::Tool);
        let message = &mut self.messages[index];
        message.content = result.content_text();
        message.tool_call_id = Some(result.tool_use_id.clone());
        message.permission_denied = result.permission_denied;
        // A refused call never ran: it is an error however it is marked.
        message.is_error = result.is_error || result.permission_denied;
        self.enter(ExecutionState::Thinking);
    }

    fn job(&mut self, task_id: &str) -> &mut BackgroundJob {
        self.background_jobs
            .entry(task_id.to_string())
            .or_insert_with(|| BackgroundJob {
                task_id: task_id.to_string(),
                tool_use_id: None,
                description: None,
                is_backgrounded: false,
                status: None,
            })
    }

    fn task_started(&mut self, task: &TaskStarted) {
        let job = self.job(&task.task_id);
        job.tool_use_id = task.tool_use_id.clone();
        job.description = task.description.clone();
        job.is_backgrounded = task.is_backgrounded;
    }

    fn task_notification(&mut self, note: &TaskNotification) {
        let job = self.job(&note.task_id);
        job.status = note.status.clone();
        job.tool_use_id = job.tool_use_id.take().or_else(|| note.tool_use_id.clone());
    }

    fn record_rate_limit(&mut self, info: &RateLimitInfo) {
        if info.status.warrants_warning() {
            self.admission_warnings.push(info.clone());
        }
        self.rate_limit = Some(info.clone());
    }

    fn result(&mut self, result: &ResultEvent) -> TurnOutcome {
        let turn = self.turns.len();
        let assistant_error = self.turn_assistant_error.take();
        let outcome = TurnOutcome {
            end: TurnEnd::classify(result, assistant_error.as_deref()),
            stop_reason: result.stop_reason.as_deref().map(StopReason::parse),
            usage: self.ledger.record(result),
            num_turns: result.num_turns,
            duration_ms: result.duration_ms,
        };
        self.guardrail_audit.extend(
            result
                .permission_denials
                .iter()
                .map(|denial| audit(denial, turn)),
        );
        if let Some(text) = &result.result_text {
            self.report = Some(FinalReport {
                content: text.clone(),
                message_ordinal: self.last_assistant_text_ordinal(),
            });
        }
        self.turns.push(outcome.clone());
        self.enter(ExecutionState::Idle);
        outcome
    }

    fn last_assistant_text_ordinal(&self) -> Option<u64> {
        self.messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::Assistant && !m.content.is_empty())
            .map(|m| m.ordinal)
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
        let count = |role| self.messages.iter().filter(|m| m.role == role).count();
        SessionTotals {
            session_key: self.init.as_ref().and_then(|i| i.session_id.clone()),
            model: self.init.as_ref().and_then(|i| i.model.clone()),
            user_messages: count(MessageRole::User),
            assistant_messages: count(MessageRole::Assistant),
            tool_calls: self.messages.iter().map(|m| m.tool_calls.len()).sum(),
            tool_results: count(MessageRole::Tool),
            tokens: self.ledger.cumulative_tokens(),
            cost_micro_usd: self.ledger.total_cost_micro_usd(),
            turns: self.turns.len(),
        }
    }
}

fn audit(denial: &PermissionDenial, turn: usize) -> GuardrailDenial {
    GuardrailDenial {
        tool_name: denial.tool_name.clone(),
        tool_use_id: denial.tool_use_id.clone(),
        tool_input: denial.tool_input.clone(),
        turn,
    }
}

#[cfg(test)]
#[path = "projection_tests.rs"]
mod tests;
