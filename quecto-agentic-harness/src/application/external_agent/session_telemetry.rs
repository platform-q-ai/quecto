//! What the member session measures for its event log (#2304): each tool
//! call from its `tool_use` to its result, the process's `system/init`,
//! the stream's unreadable lines and unknown events, and each turn as it
//! ended. Pure: [`super::session_core::SessionCore`] holds one and folds
//! every event through it; time is the session clock's.

use std::collections::{BTreeMap, VecDeque};

use super::dto::{AgentClockInstant, SessionRecord, TurnOutcome};
use crate::domain::external_agent::stream::{
    AssistantContent, ExternalAgentEvent, SkippedLineReason, ToolResultEvent,
};
use crate::domain::external_agent::telemetry::{
    ExternalAgentStreamDiagnostic, ExternalAgentTool, ExternalAgentTurn, board_task_id,
    error_reason_kind, recorded_name, tool_summary,
};
use crate::domain::external_agent::turn::{FailureKind, TurnEnd};

/// The most tool calls awaiting their result the session tracks; past it
/// the oldest is recorded unanswered.
pub const PENDING_TOOL_CAPACITY: usize = 256;

/// The most distinct diagnostic kinds counted; past it new ones are not.
pub const DIAGNOSTIC_KIND_CAPACITY: usize = 64;

/// The `terminal_reason` of a turn that spent its budget.
const BUDGET_EXHAUSTED: &str = "budget_exhausted";

/// How a turn ended, for its record.
#[derive(Debug, Clone, Copy)]
pub(crate) enum TurnCut<'a> {
    /// The session settled it: on its result (`Some`), or on an accepted
    /// interrupt that left none (`None`: `aborted`).
    Settled(Option<&'a TurnOutcome>),
    /// Its process's output ended first: `exited`.
    Exited,
    /// The member was closed while it ran: `closed`.
    Closed,
    /// The member was ended while it ran, its state unknown: `abandoned`.
    Abandoned,
}

impl TurnCut<'_> {
    /// The turn's end kind and reason kind, as its record keeps them.
    fn kinds(self) -> (&'static str, Option<String>) {
        match self {
            TurnCut::Settled(Some(outcome)) => end_kind(&outcome.end),
            TurnCut::Settled(None) => ("aborted", None),
            TurnCut::Exited => ("exited", None),
            TurnCut::Closed => ("closed", None),
            TurnCut::Abandoned => ("abandoned", None),
        }
    }
}

#[derive(Debug)]
struct PendingTool {
    record: ExternalAgentTool,
    called_at: AgentClockInstant,
}

/// The session's measurements.
#[derive(Debug, Default)]
pub(crate) struct SessionTelemetry {
    pending: VecDeque<PendingTool>,
    diagnostics: BTreeMap<(&'static str, String), u64>,
    claude_session_id: Option<String>,
    model: Option<String>,
    /// The process started and its first `system/init` has not come yet.
    awaiting_init: bool,
    started_at: Option<AgentClockInstant>,
}

impl SessionTelemetry {
    /// A process started at `now`: its first init is recorded, and
    /// nothing of the last one's is carried over (its open calls, its
    /// session id, its model). The diagnostic counts are the session's.
    pub(crate) fn process_started(&mut self, now: AgentClockInstant) {
        self.pending.clear();
        self.claude_session_id = None;
        self.model = None;
        self.started_at = Some(now);
        self.awaiting_init = true;
    }

    /// Milliseconds since the process started, if it did.
    pub(crate) fn wall_ms(&self, now: AgentClockInstant) -> Option<u64> {
        wall_ms_since(self.started_at, now)
    }

    /// When the process started, if it did.
    pub(crate) fn started_at(&self) -> Option<AgentClockInstant> {
        self.started_at
    }

    /// Measure one event of running turn `turn`, read at `now`.
    pub(crate) fn observe(
        &mut self,
        event: &ExternalAgentEvent,
        turn: Option<u64>,
        now: AgentClockInstant,
        records: &mut Vec<SessionRecord>,
    ) {
        match event {
            ExternalAgentEvent::Init(init) => {
                self.claude_session_id = init.session_id.as_deref().map(recorded_name);
                self.model = init.model.as_deref().map(recorded_name);
                if std::mem::take(&mut self.awaiting_init) {
                    records.push(SessionRecord::Initialized {
                        cli_version: init.cli_version.as_deref().map(recorded_name),
                        claude_session_id: self.claude_session_id.clone(),
                        model: self.model.clone(),
                    });
                }
            }
            ExternalAgentEvent::AssistantBlock {
                block: AssistantContent::ToolUse { id, name, input },
                ..
            } => self.called(turn, (id, name, input), now, records),
            ExternalAgentEvent::ToolResult(result) => self.answered(result, now, records),
            ExternalAgentEvent::Unknown { kind } => {
                self.diagnose("unknown_event", kind, None, turn, records);
            }
            ExternalAgentEvent::LineSkipped(line) => {
                let reason = match line.reason {
                    SkippedLineReason::OverCap => "over_cap",
                    SkippedLineReason::NotUtf8 => "not_utf8",
                };
                self.diagnose("skipped_line", reason, Some(line.bytes), turn, records);
            }
            // They feed `external_agent_turn` (the table's word), through
            // the projection's outcome, when the session ends the turn
            // (`turn_ended`): the assistant's error kind names why it
            // failed.
            ExternalAgentEvent::AssistantError { .. } | ExternalAgentEvent::Result(_) => {}
            // Ignored, as `stream_telemetry` says; the session's tests
            // feed every event of its table through `fold` to hold the two
            // together.
            ExternalAgentEvent::ThinkingTokens { .. }
            | ExternalAgentEvent::AssistantBlock { .. }
            | ExternalAgentEvent::UserText { .. }
            | ExternalAgentEvent::TaskStarted(_)
            | ExternalAgentEvent::TaskNotification(_)
            | ExternalAgentEvent::BackgroundTasksChanged { .. }
            | ExternalAgentEvent::RateLimit(_)
            | ExternalAgentEvent::InterruptAnswered(_) => {}
        }
    }

    fn called(
        &mut self,
        turn: Option<u64>,
        (id, name, input): (&str, &str, &serde_json::Value),
        now: AgentClockInstant,
        records: &mut Vec<SessionRecord>,
    ) {
        if self.pending.len() == PENDING_TOOL_CAPACITY
            && let Some(oldest) = self.pending.pop_front()
        {
            records.push(finished(oldest, "unanswered", 0, now));
        }
        self.pending.push_back(PendingTool {
            record: ExternalAgentTool {
                member_turn: turn,
                tool: recorded_name(name),
                tool_use_id: recorded_name(id),
                duration_ms: 0,
                outcome: String::new(),
                rule_id: None,
                argument_bytes: input.to_string().len(),
                result_bytes: 0,
                summary: tool_summary(name, input),
                task_id: board_task_id(name, input),
            },
            called_at: now,
        });
        assert!(self.pending.len() <= PENDING_TOOL_CAPACITY);
    }

    /// A result answers the call it names, else the oldest open one.
    fn answered(
        &mut self,
        result: &ToolResultEvent,
        now: AgentClockInstant,
        records: &mut Vec<SessionRecord>,
    ) {
        let position = match &result.tool_use_id {
            Some(id) => {
                let id = recorded_name(id);
                self.pending
                    .iter()
                    .position(|pending| pending.record.tool_use_id == id)
            }
            None => self.pending.front().map(|_| 0),
        };
        let Some(pending) = position.and_then(|at| self.pending.remove(at)) else {
            return;
        };
        let outcome = match (result.permission_denied, result.is_error) {
            (true, _) => "denied",
            (false, true) => "error",
            (false, false) => "ok",
        };
        let bytes = result.content_text().len();
        records.push(finished(pending, outcome, bytes, now));
    }

    fn diagnose(
        &mut self,
        kind: &'static str,
        name: &str,
        bytes: Option<usize>,
        turn: Option<u64>,
        records: &mut Vec<SessionRecord>,
    ) {
        let key = (kind, recorded_name(name));
        let room = self.diagnostics.len() < DIAGNOSTIC_KIND_CAPACITY;
        let count = match (self.diagnostics.get_mut(&key), room) {
            (Some(count), _) => {
                *count += 1;
                *count
            }
            (None, true) => {
                self.diagnostics.insert(key.clone(), 1);
                1
            }
            (None, false) => return,
        };
        if count.is_power_of_two() {
            records.push(SessionRecord::StreamDiagnostic(
                ExternalAgentStreamDiagnostic {
                    kind: kind.to_string(),
                    name: key.1,
                    bytes,
                    count,
                    member_turn: turn,
                },
            ));
        }
    }

    /// Every call still open is recorded unanswered: its turn, or the
    /// member, ended first.
    pub(crate) fn calls_cut(&mut self, now: AgentClockInstant, records: &mut Vec<SessionRecord>) {
        while let Some(pending) = self.pending.pop_front() {
            records.push(finished(pending, "unanswered", 0, now));
        }
        assert!(self.pending.is_empty());
    }

    /// Turn `turn` ended as `cut` says: its calls still open are recorded
    /// unanswered, then the turn.
    pub(crate) fn turn_ended(
        &mut self,
        turn: u64,
        cut: TurnCut<'_>,
        now: AgentClockInstant,
        records: &mut Vec<SessionRecord>,
    ) {
        self.calls_cut(now, records);
        let outcome = match cut {
            TurnCut::Settled(outcome) => outcome,
            TurnCut::Exited | TurnCut::Closed | TurnCut::Abandoned => None,
        };
        let (turn_end, reason_kind) = cut.kinds();
        let tokens = outcome.map(|o| o.usage.tokens).unwrap_or_default();
        records.push(SessionRecord::TurnReported(Box::new(ExternalAgentTurn {
            member_turn: turn,
            turn_end: turn_end.to_string(),
            reason_kind,
            claude_session_id: self.claude_session_id.clone(),
            model: self.model.clone(),
            is_error: outcome.and_then(|o| o.is_error),
            num_turns: outcome.and_then(|o| o.num_turns),
            duration_ms: outcome.and_then(|o| o.duration_ms),
            duration_api_ms: outcome.and_then(|o| o.duration_api_ms),
            input_tokens: tokens.input,
            output_tokens: tokens.output,
            cache_read_tokens: tokens.cache_read,
            cache_write_tokens: tokens.cache_write,
            list_price_cost_micro_usd: outcome.map_or(0, |o| o.usage.cost_micro_usd),
            list_price_total_micro_usd: outcome.map_or(0, |o| o.usage.total_cost_micro_usd),
            task_id: None,
            cost_drop: outcome.and_then(|o| o.usage.cost_drop),
        })));
    }
}

fn finished(
    pending: PendingTool,
    outcome: &str,
    result_bytes: usize,
    now: AgentClockInstant,
) -> SessionRecord {
    let mut record = pending.record;
    record.outcome = outcome.to_string();
    record.result_bytes = result_bytes;
    record.duration_ms = now.0.saturating_sub(pending.called_at.0);
    SessionRecord::ToolFinished(Box::new(record))
}

/// Milliseconds from `started_at`, if the process started, to `now`.
pub(crate) fn wall_ms_since(
    started_at: Option<AgentClockInstant>,
    now: AgentClockInstant,
) -> Option<u64> {
    started_at.map(|start| now.0.saturating_sub(start.0))
}

/// A turn end's kind and, for one that did not complete, why: its
/// `terminal_reason`, unless that is only the generic
/// [`GENERIC_TERMINAL_REASON`], which the HTTP status (`api_<status>`) or
/// the assistant's error kind names better; then the first error's words;
/// the generic word last.
fn end_kind(end: &TurnEnd) -> (&'static str, Option<String>) {
    let failure = match end {
        TurnEnd::Completed => return ("completed", None),
        TurnEnd::Failed(failure) => failure,
    };
    let specific_reason = match failure.terminal_reason.as_deref() {
        Some(GENERIC_TERMINAL_REASON) | None => None,
        Some(reason) => Some(reason),
    };
    let reason = specific_reason
        .map(recorded_name)
        .or_else(|| {
            failure
                .api_error_status
                .map(|status| format!("api_{status}"))
        })
        .or_else(|| failure.assistant_error.as_deref().map(recorded_name))
        .or_else(|| error_reason_kind(&failure.errors))
        .or_else(|| failure.terminal_reason.as_deref().map(recorded_name));
    let kind = match (failure.kind, failure.terminal_reason.as_deref()) {
        (FailureKind::Aborted, _) => "aborted",
        (FailureKind::Error, Some(BUDGET_EXHAUSTED)) => "budget_exceeded",
        (FailureKind::Error, _) => "failed",
    };
    (kind, reason)
}

/// The `terminal_reason` that says only that the API failed: the status or
/// the assistant's error says how.
const GENERIC_TERMINAL_REASON: &str = "api_error";

#[cfg(test)]
#[path = "session_telemetry_tests.rs"]
mod tests;
