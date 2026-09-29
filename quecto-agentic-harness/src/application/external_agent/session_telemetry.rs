//! What the member session measures for its event log (#2304): each tool
//! call from its `tool_use` to its result, the process's `system/init`,
//! the stream's unreadable lines and unknown events, and each turn as it
//! ended. Pure: [`super::session_core::SessionCore`] holds one and folds
//! every event through it; time is the session clock's.
#![allow(dead_code, unused_imports)] // red-phase stub (#2304)

use std::collections::{BTreeMap, VecDeque};

use super::dto::{AgentClockInstant, SessionRecord, TurnOutcome};
use crate::domain::external_agent::stream::{
    AssistantContent, ExternalAgentEvent, SkippedLineReason, ToolResultEvent,
};
use crate::domain::external_agent::telemetry::{
    ExternalAgentStreamDiagnostic, ExternalAgentTool, ExternalAgentTurn, board_task_id,
    recorded_name, tool_summary,
};
use crate::domain::external_agent::turn::{FailureKind, TurnEnd};

/// The most tool calls awaiting their result the session tracks; past it
/// the oldest is recorded unanswered.
pub const PENDING_TOOL_CAPACITY: usize = 256;

/// The most distinct diagnostic kinds counted; past it new ones are not.
pub const DIAGNOSTIC_KIND_CAPACITY: usize = 64;

/// The `terminal_reason` of a turn that spent its budget.
const BUDGET_EXHAUSTED: &str = "budget_exhausted";

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
    /// A `system/init` has been seen since the process started.
    initialized: bool,
    started_at: Option<AgentClockInstant>,
}

impl SessionTelemetry {
    /// A process started at `now`: its first init is recorded.
    pub(crate) fn process_started(&mut self, now: AgentClockInstant) {
        self.started_at = Some(now);
        self.initialized = false;
    }

    /// Milliseconds since the process started, if it did.
    pub(crate) fn wall_ms(&self, now: AgentClockInstant) -> Option<u64> {
        let _ = (now, self.started_at);
        None
    }

    /// Measure one event of running turn `turn`, read at `now`.
    pub(crate) fn observe(
        &mut self,
        event: &ExternalAgentEvent,
        turn: Option<u64>,
        now: AgentClockInstant,
        records: &mut Vec<SessionRecord>,
    ) {
        let _ = (event, turn, now, records);
    }

    fn called(
        &mut self,
        turn: Option<u64>,
        id: &str,
        name: &str,
        input: &serde_json::Value,
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
            None => (!self.pending.is_empty()).then_some(0),
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

    /// Turn `turn` ended with `outcome` (`None`: its process exited, or it
    /// was stopped without a result): its calls still open are recorded
    /// unanswered, then the turn.
    pub(crate) fn turn_ended(
        &mut self,
        turn: u64,
        outcome: Option<&TurnOutcome>,
        exited: bool,
        now: AgentClockInstant,
        records: &mut Vec<SessionRecord>,
    ) {
        let _ = (turn, outcome, exited, now, records);
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

/// A turn end's kind and, for one that did not complete, why.
fn end_kind(end: &TurnEnd) -> (&'static str, Option<String>) {
    let TurnEnd::Failed(failure) = end else {
        return ("completed", None);
    };
    let reason = failure
        .terminal_reason
        .as_deref()
        .map(recorded_name)
        .or_else(|| {
            failure
                .api_error_status
                .map(|status| format!("api_{status}"))
        })
        .or_else(|| failure.assistant_error.as_deref().map(recorded_name));
    let kind = match (failure.kind, failure.terminal_reason.as_deref()) {
        (FailureKind::Aborted, _) => "aborted",
        (FailureKind::Error, Some(BUDGET_EXHAUSTED)) => "budget_exceeded",
        (FailureKind::Error, _) => "failed",
    };
    (kind, reason)
}

#[cfg(test)]
#[path = "session_telemetry_tests.rs"]
mod tests;
