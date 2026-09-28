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

use super::dto::audit::input_preview;
use super::dto::{
    BackgroundJob, ExecutionState, FinalReport, GuardrailDenial, MessageRole, ProjectedMessage,
    ProjectedToolCall, ProjectionStep, SessionTotals, TurnOutcome,
};
use crate::domain::external_agent::stream::{
    AssistantContent, ExternalAgentEvent, InitEvent, PermissionDenial, RateLimitInfo, ResultEvent,
    TaskNotification, TaskStarted, ToolResultEvent,
};
use crate::domain::external_agent::turn::TurnEnd;
use crate::domain::external_agent::usage::UsageLedger;
use crate::domain::message::StopReason;

/// Folds [`ExternalAgentEvent`]s into the member's protocol values.
#[derive(Debug, Default)]
pub struct Projector {
    state: ExecutionState,
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

    /// Fold one event: what it changed.
    pub fn apply(&mut self, event: &ExternalAgentEvent) -> ProjectionStep {
        let before = self.state;
        let turn_end = self.fold(event);
        ProjectionStep {
            transition: (self.state != before).then_some(self.state),
            turn_end,
        }
    }

    fn fold(&mut self, event: &ExternalAgentEvent) -> Option<TurnOutcome> {
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
        message.tool_call_id = result.tool_use_id.clone();
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
            warnings: Vec::new(),
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
                failure: None,
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

    pub fn messages(&self) -> &[ProjectedMessage] {
        &self.messages
    }

    /// The latest `system/init`.
    pub fn init(&self) -> Option<&InitEvent> {
        self.init.as_ref()
    }

    /// The latest turn's outcome.
    pub fn last_turn(&self) -> Option<&TurnOutcome> {
        self.turns.last()
    }

    pub fn report(&self) -> Option<&FinalReport> {
        self.report.as_ref()
    }

    /// The latest rate-limit standing.
    pub fn rate_limit(&self) -> Option<&RateLimitInfo> {
        self.rate_limit.as_ref()
    }

    /// The latest rate-limit events whose status warranted a warning,
    /// oldest first.
    pub fn admission_warnings(&self) -> impl Iterator<Item = &RateLimitInfo> {
        self.admission_warnings.iter()
    }

    /// The latest permission denials, oldest first.
    pub fn guardrail_audit(&self) -> impl Iterator<Item = &GuardrailDenial> {
        self.guardrail_audit.iter()
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
            guardrail_denials: self.guardrail_audit.len(),
            admission_warnings: self.admission_warnings.len(),
            unknown_events: self.unknown_events,
        }
    }
}

fn audit(denial: &PermissionDenial, turn: usize) -> GuardrailDenial {
    GuardrailDenial {
        tool_name: denial.tool_name.clone(),
        tool_use_id: denial.tool_use_id.clone(),
        tool_input_preview: input_preview(&denial.tool_input),
        turn,
    }
}

#[cfg(test)]
#[path = "projection_bounds_tests.rs"]
mod bounds_tests;
#[cfg(test)]
#[path = "projection_tests.rs"]
mod tests;
