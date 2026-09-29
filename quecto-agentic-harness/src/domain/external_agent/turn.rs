//! How an external agent's turn ended, and what it used (#2285).
//!
//! The traps spike #2264 found live here: a `result` whose `subtype` says
//! `success` can still be an error, so a turn is [`TurnEnd::Completed`]
//! only on `terminal_reason == "completed"` with `is_error == false`; and
//! `result.usage` is per turn while `total_cost_usd` is cumulative, so a
//! turn's cost is the delta of the cumulative total, kept in whole
//! micro-USD and rounded once per result.

use super::stream::ResultEvent;

/// The `terminal_reason` of a turn that finished its work.
pub const TERMINAL_REASON_COMPLETED: &str = "completed";

/// How one turn ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnEnd {
    Completed,
    Failed(TurnFailure),
}

/// Why a turn did not complete: every signal the stream gave.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TurnFailure {
    pub terminal_reason: Option<String>,
    pub api_error_status: Option<u16>,
    /// The `error` an assistant event of the turn carried
    /// (`authentication_failed` …).
    pub assistant_error: Option<String>,
    /// The result's `errors[]`.
    pub errors: Vec<String>,
    /// Whether the turn was stopped (an abort) or went wrong.
    pub kind: FailureKind,
}

/// How a failed turn failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FailureKind {
    /// The turn went wrong: an API error, a limit, a malformed result.
    #[default]
    Error,
    /// The turn was stopped before it completed: interrupted, stopped by
    /// a hook, or deferred (see [`ABORT_TERMINAL_REASONS`]).
    Aborted,
}

impl FailureKind {
    /// An abort when the `terminal_reason` is one of
    /// [`ABORT_TERMINAL_REASONS`]; otherwise an error.
    pub fn of(terminal_reason: Option<&str>) -> Self {
        match terminal_reason {
            Some(reason) if ABORT_TERMINAL_REASONS.contains(&reason) => Self::Aborted,
            _ => Self::Error,
        }
    }
}

/// The `terminal_reason`s the CLI treats as a stop, not an error. They are
/// still failed turns (only `completed` completes one), classified as
/// [`FailureKind::Aborted`] so an abort is shown as one.
pub const ABORT_TERMINAL_REASONS: &[&str] = &[
    "aborted_streaming",
    "aborted_tools",
    "hook_stopped",
    "stop_hook_prevented",
    "tool_deferred",
    "background_requested",
];

impl TurnFailure {
    /// A one-line account of the failure, for a report that has no text.
    pub fn describe(&self) -> String {
        let signals: Vec<String> = [
            self.terminal_reason
                .as_ref()
                .map(|reason| format!("terminal_reason: {reason}")),
            self.api_error_status
                .map(|status| format!("api_error_status: {status}")),
            self.assistant_error
                .as_ref()
                .map(|kind| format!("assistant error: {kind}")),
        ]
        .into_iter()
        .flatten()
        .collect();
        let mut text = String::from(match self.kind {
            FailureKind::Aborted => "the turn was aborted",
            FailureKind::Error => "the turn failed",
        });
        if let [_, ..] = signals.as_slice() {
            text.push_str(&format!(" ({})", signals.join("; ")));
        }
        if let [_, ..] = self.errors.as_slice() {
            text.push_str(&format!(": {}", self.errors.join("; ")));
        }
        text
    }
}

impl TurnEnd {
    /// Classify a turn from its `result` and any assistant error the turn
    /// carried. Completed only when the stream affirms both halves:
    /// `terminal_reason` is `completed` and `is_error` is `false`; a
    /// missing field is a failure. The result is followed over an
    /// assistant `error` (the projection records that as a warning), which
    /// a failure carries. `subtype` is never consulted.
    pub fn classify(result: &ResultEvent, assistant_error: Option<&str>) -> Self {
        match (result.terminal_reason.as_deref(), result.is_error) {
            (Some(TERMINAL_REASON_COMPLETED), Some(false)) => Self::Completed,
            _ => Self::Failed(TurnFailure {
                terminal_reason: result.terminal_reason.clone(),
                api_error_status: result.api_error_status,
                assistant_error: assistant_error.map(str::to_string),
                errors: result.errors.clone(),
                kind: FailureKind::of(result.terminal_reason.as_deref()),
            }),
        }
    }

    pub fn is_completed(&self) -> bool {
        matches!(self, Self::Completed)
    }
}

#[cfg(test)]
#[path = "turn_tests.rs"]
mod tests;
