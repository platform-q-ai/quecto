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
//!   usage recorded ([`UsageLedger`]), and it becomes the final report: its
//!   text, or for a failed turn without text, the failure.
//! - `system/init` repeats every turn and never resets what was folded.
//! - Nothing but the conversation grows with the stream: each step's
//!   transition and turn end are returned, not kept; the audit, the
//!   admission warnings and the jobs are bounded, with counters.

use std::collections::{HashMap, VecDeque};

use super::dto::audit::input_preview;
use super::dto::{
    ADMISSION_WARNING_CAPACITY, BACKGROUND_JOB_CAPACITY, BackgroundJob, ExecutionState,
    FinalReport, GUARDRAIL_AUDIT_CAPACITY, GuardrailDenial, MessageRole, ProjectedMessage,
    ProjectedToolCall, ProjectionStep, SessionTotals, TOOL_RESULT_CONTENT_BYTES, TRUNCATION_MARKER,
    TurnOutcome, TurnWarning,
};
use crate::domain::conversation::value_objects::message::StopReason;
use crate::domain::external_agent::stream::{
    AssistantContent, BackgroundTask, ExternalAgentEvent, InitEvent, PermissionDenial,
    RateLimitInfo, ResultEvent, TaskNotification, TaskStarted, ToolResultEvent,
};
use crate::domain::external_agent::turn::TurnEnd;
use crate::domain::external_agent::usage::UsageLedger;

/// Folds [`ExternalAgentEvent`]s into the member's protocol values.
#[derive(Debug, Default)]
pub struct Projector {
    state: ExecutionState,
    messages: Vec<ProjectedMessage>,
    /// This turn's API messages by id (message ids do not span turns).
    turn_messages: HashMap<String, usize>,
    /// Where this turn's messages start: its report points only at them.
    turn_start: usize,
    /// This turn's tool calls without a result yet, oldest first.
    open_tool_calls: VecDeque<String>,
    init: Option<InitEvent>,
    ledger: UsageLedger,
    turns: usize,
    last_turn: Option<TurnOutcome>,
    report: Option<FinalReport>,
    turn_assistant_error: Option<String>,
    rate_limit: Option<RateLimitInfo>,
    admission_warnings: VecDeque<RateLimitInfo>,
    admission_warning_count: usize,
    guardrail_audit: VecDeque<GuardrailDenial>,
    guardrail_denial_count: usize,
    background_jobs: VecDeque<BackgroundJob>,
    unknown_events: usize,
    skipped_lines: usize,
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

    /// A new agent process started (the session calls this when it spawns
    /// one): its cumulative totals start from zero.
    pub fn process_started(&mut self) {
        self.ledger.process_started();
        // The old process's tasks ended with it (a killed process's
        // background job never reports), and a new process may reuse
        // their ids.
        self.background_jobs.clear();
        self.end_turn_state();
        self.state = ExecutionState::Idle;
    }

    /// Forget what belongs to the turn that ended (or to a process that
    /// ended): the next turn starts after the current messages.
    fn end_turn_state(&mut self) {
        self.turn_messages.clear();
        self.open_tool_calls.clear();
        self.turn_assistant_error = None;
        self.turn_start = self.messages.len();
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
                self.state = ExecutionState::Thinking;
            }
            ExternalAgentEvent::ThinkingTokens { .. } => self.state = ExecutionState::Thinking,
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
            ExternalAgentEvent::BackgroundTasksChanged { tasks } => self.background_set(tasks),
            ExternalAgentEvent::RateLimit(info) => self.record_rate_limit(info),
            ExternalAgentEvent::Result(result) => return Some(self.result(result)),
            // Which user turns an interrupt withdrew is the session's
            // bookkeeping (#2287): nothing of the conversation changes.
            ExternalAgentEvent::InterruptAnswered(_) => {}
            ExternalAgentEvent::Unknown { .. } => self.unknown_events += 1,
            // Only counted here: whether a skipped line ends the turn is
            // the session's call (S3), which sees the event itself.
            ExternalAgentEvent::LineSkipped(_) => self.skipped_lines += 1,
        }
        None
    }

    fn push(&mut self, role: MessageRole) -> usize {
        let ordinal = self.messages.len() as u64;
        self.messages.push(ProjectedMessage::new(ordinal, role));
        self.messages.len() - 1
    }

    fn assistant_block(&mut self, message_id: &str, block: &AssistantContent) {
        let index = match self.turn_messages.get(message_id) {
            Some(index) => *index,
            None => {
                let index = self.push(MessageRole::Assistant);
                self.messages[index].api_message_id = Some(message_id.to_string());
                self.turn_messages.insert(message_id.to_string(), index);
                index
            }
        };
        let message = &mut self.messages[index];
        assert_eq!(message.role, MessageRole::Assistant);
        self.state = match block {
            AssistantContent::Text(text) => {
                message.content.push_str(text);
                ExecutionState::Streaming
            }
            AssistantContent::ToolUse { id, name, input } => {
                message.tool_calls.push(ProjectedToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: input.clone(),
                });
                self.open_tool_calls.push_back(id.clone());
                ExecutionState::RunningTool
            }
            AssistantContent::Thinking { text } => {
                if let Some(text) = text {
                    message
                        .thinking
                        .get_or_insert_with(String::new)
                        .push_str(text);
                }
                ExecutionState::Thinking
            }
        };
    }

    fn tool_result(&mut self, result: &ToolResultEvent) {
        // A result names its call; one that does not closes the oldest
        // call still open, so the call is never left running.
        let call = match &result.tool_use_id {
            Some(id) => {
                if let Some(position) = self.open_tool_calls.iter().position(|open| open == id) {
                    self.open_tool_calls.remove(position);
                }
                Some(id.clone())
            }
            None => self.open_tool_calls.pop_front(),
        };
        let index = self.push(MessageRole::Tool);
        let message = &mut self.messages[index];
        let (content, truncated_from_bytes) = bounded_tool_content(result.content_text());
        message.content = content;
        message.truncated_from_bytes = truncated_from_bytes;
        message.tool_call_id = call;
        message.permission_denied = result.permission_denied;
        // A refused call never ran: it is an error however it is marked.
        message.is_error = result.is_error || result.permission_denied;
        self.state = ExecutionState::Thinking;
    }

    /// The job `task_id`, tracked from now on: the oldest is dropped past
    /// [`BACKGROUND_JOB_CAPACITY`].
    fn job(&mut self, task_id: &str) -> &mut BackgroundJob {
        let position = match self
            .background_jobs
            .iter()
            .position(|j| j.task_id == task_id)
        {
            Some(position) => position,
            None => {
                if self.background_jobs.len() == BACKGROUND_JOB_CAPACITY {
                    // The oldest finished job goes first; a running one
                    // only when none has finished.
                    let evicted = self
                        .background_jobs
                        .iter()
                        .position(BackgroundJob::is_finished)
                        .unwrap_or(0);
                    self.background_jobs.remove(evicted);
                }
                self.background_jobs.push_back(BackgroundJob {
                    task_id: task_id.to_string(),
                    tool_use_id: None,
                    description: None,
                    is_backgrounded: false,
                    status: None,
                });
                self.background_jobs.len() - 1
            }
        };
        assert!(self.background_jobs.len() <= BACKGROUND_JOB_CAPACITY);
        &mut self.background_jobs[position]
    }

    /// A started task is a new running task, even under a reused id: a
    /// finished record is replaced, not updated. A record that is not
    /// finished (listed by `background_tasks_changed` just before) keeps
    /// its description and background flag when the event has none.
    fn task_started(&mut self, task: &TaskStarted) {
        let job = self.job(&task.task_id);
        let listed = match job.is_finished() {
            true => (None, false),
            false => (job.description.take(), job.is_backgrounded),
        };
        job.status = None;
        job.tool_use_id = task.tool_use_id.clone();
        job.description = task.description.clone().or(listed.0);
        job.is_backgrounded = task.is_backgrounded || listed.1;
    }

    fn task_notification(&mut self, note: &TaskNotification) {
        let job = self.job(&note.task_id);
        job.status = note.status.clone();
        job.tool_use_id = job.tool_use_id.take().or_else(|| note.tool_use_id.clone());
    }

    /// `background_tasks_changed` is the full current background list: a
    /// background job not on it has ended. Foreground tasks are untouched.
    fn background_set(&mut self, tasks: &[BackgroundTask]) {
        let listed = |id: &str| tasks.iter().any(|task| task.task_id == id);
        self.background_jobs
            .retain(|job| match job.is_backgrounded {
                true => listed(&job.task_id),
                false => true,
            });
        for task in tasks {
            let job = self.job(&task.task_id);
            job.is_backgrounded = true;
            job.description = task.description.clone().or(job.description.take());
        }
    }

    fn record_rate_limit(&mut self, info: &RateLimitInfo) {
        if info.status.warrants_warning() {
            self.admission_warning_count += 1;
            push_bounded(
                &mut self.admission_warnings,
                info.clone(),
                ADMISSION_WARNING_CAPACITY,
            );
        }
        self.rate_limit = Some(info.clone());
    }

    fn result(&mut self, result: &ResultEvent) -> TurnOutcome {
        let turn = self.turns;
        let assistant_error = self.turn_assistant_error.take();
        let end = TurnEnd::classify(result, assistant_error.as_deref());
        let usage = self.ledger.record(result);
        let mut warnings = Vec::new();
        if let (TurnEnd::Completed, Some(kind)) = (&end, &assistant_error) {
            warnings.push(TurnWarning::AssistantError(kind.clone()));
        }
        if let Some(drop) = usage.cost_drop {
            warnings.push(TurnWarning::CumulativeCostDropped {
                previous_micro_usd: drop.previous_micro_usd,
                reported_micro_usd: drop.reported_micro_usd,
            });
        }
        for denial in &result.permission_denials {
            self.guardrail_denial_count += 1;
            push_bounded(
                &mut self.guardrail_audit,
                audit(denial, turn),
                GUARDRAIL_AUDIT_CAPACITY,
            );
        }
        self.report = Some(self.report_of(result, &end));
        let outcome = TurnOutcome {
            end,
            stop_reason: result.stop_reason.as_deref().map(StopReason::parse),
            usage,
            num_turns: result.num_turns,
            duration_ms: result.duration_ms,
            warnings,
        };
        self.turns += 1;
        self.last_turn = Some(outcome.clone());
        self.end_turn_state();
        self.state = ExecutionState::Idle;
        outcome
    }

    /// The turn's report: its `result` text, or for a failed turn without
    /// one, the failure; never an earlier turn's answer.
    fn report_of(&self, result: &ResultEvent, end: &TurnEnd) -> FinalReport {
        let failure = match end {
            TurnEnd::Failed(failure) => Some(failure.clone()),
            TurnEnd::Completed => None,
        };
        let content = match (&result.result_text, &failure) {
            (Some(text), _) => text.clone(),
            (None, Some(failure)) => failure.describe(),
            (None, None) => String::new(),
        };
        FinalReport {
            content,
            message_ordinal: self.last_assistant_text_ordinal(),
            failure,
        }
    }

    /// The last assistant message with text of the current turn.
    fn last_assistant_text_ordinal(&self) -> Option<u64> {
        assert!(self.turn_start <= self.messages.len());
        self.messages[self.turn_start..]
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
        self.last_turn.as_ref()
    }

    pub fn report(&self) -> Option<&FinalReport> {
        self.report.as_ref()
    }

    /// The latest rate-limit standing.
    pub fn rate_limit(&self) -> Option<&RateLimitInfo> {
        self.rate_limit.as_ref()
    }

    /// The latest rate-limit events whose status warranted a warning,
    /// oldest first (at most [`ADMISSION_WARNING_CAPACITY`]).
    pub fn admission_warnings(&self) -> impl Iterator<Item = &RateLimitInfo> {
        self.admission_warnings.iter()
    }

    /// The latest permission denials, oldest first (at most
    /// [`GUARDRAIL_AUDIT_CAPACITY`]).
    pub fn guardrail_audit(&self) -> impl Iterator<Item = &GuardrailDenial> {
        self.guardrail_audit.iter()
    }

    /// The tracked jobs, oldest first (at most [`BACKGROUND_JOB_CAPACITY`]).
    pub fn background_jobs(&self) -> impl Iterator<Item = &BackgroundJob> {
        self.background_jobs.iter()
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
            turns: self.turns,
            guardrail_denials: self.guardrail_denial_count,
            admission_warnings: self.admission_warning_count,
            unknown_events: self.unknown_events,
            skipped_lines: self.skipped_lines,
        }
    }
}

/// A tool result's content as stored: whole up to
/// [`TOOL_RESULT_CONTENT_BYTES`], else cut on a character boundary with
/// [`TRUNCATION_MARKER`], and the original length.
fn bounded_tool_content(mut content: String) -> (String, Option<usize>) {
    let length = content.len();
    match length <= TOOL_RESULT_CONTENT_BYTES {
        true => (content, None),
        false => {
            let mut end = TOOL_RESULT_CONTENT_BYTES;
            while !content.is_char_boundary(end) {
                let before = end;
                end -= 1;
                assert!(end < before, "backing off to a char boundary moves back");
            }
            content.truncate(end);
            content.push_str(TRUNCATION_MARKER);
            (content, Some(length))
        }
    }
}

/// Append `item`, dropping the oldest past `capacity`.
fn push_bounded<T>(items: &mut VecDeque<T>, item: T, capacity: usize) {
    assert!(capacity > 0, "a bounded history keeps at least one item");
    if items.len() == capacity {
        items.pop_front();
    }
    items.push_back(item);
    assert!(items.len() <= capacity);
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
#[path = "projection_process_tests.rs"]
mod process_tests;
#[cfg(test)]
#[path = "projection_tests.rs"]
mod tests;
