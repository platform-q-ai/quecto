//! Which gate reads a member's inference admission (#2339). The harness
//! reads the run's admission (`_request_admission`) at three gates, each
//! needed and each recorded as its own decision, so the event log counts
//! them apart:
//!
//! - [`Model`](AdmissionGate::Model): once per model request, before its
//!   first send. Each request also records its usage once
//!   (`_record_request`), which may add redeliveries of a record the
//!   store was too busy to take, with no gate read of their own.
//! - [`Retry`](AdmissionGate::Retry): before each reattempt of a request
//!   (a transient-failure retry, a stream re-initiation, a resend after an
//!   OAuth refresh), because the run may have been paused, stopped or run
//!   past its deadline while the failed send and its backoff took their
//!   time.
//! - [`Tool`](AdmissionGate::Tool): before each tool call the reply asked
//!   for, because the model took its own time to answer, and each earlier
//!   call in the batch (a long command) takes its own; and a terminal
//!   run's coordinator may still read its report, where no other tool
//!   runs.
//!
//! A member therefore reads its admission about `1 + t` times per model
//! request, `t` being the request's tool calls (plus one per reattempt).
//! No token budget warns or pauses a run (owner decision, 2026-10-14), so
//! every read records its gate.
use crate::domain::inference::value_objects::provider::RequestAttempt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionGate {
    Model,
    Retry,
    Tool,
}

impl AdmissionGate {
    /// Every gate, in the order the allowlist names them.
    pub const ALL: [Self; 3] = [Self::Model, Self::Retry, Self::Tool];

    /// The gate a model request's send is admitted at.
    pub fn for_attempt(attempt: RequestAttempt) -> Self {
        match attempt {
            RequestAttempt::First => Self::Model,
            RequestAttempt::Reattempt => Self::Retry,
        }
    }

    /// The gate's name on the board's wire (`_request_admission(gate)`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::Retry => "retry",
            Self::Tool => "tool",
        }
    }

    /// The decision the board records for a read at this gate.
    pub fn decision(self) -> &'static str {
        match self {
            Self::Model => "model_gate",
            Self::Retry => "retry_gate",
            Self::Tool => "tool_gate",
        }
    }

    /// The gate `name` names, from the allowlist only: any other text
    /// (another case, a padded name) names none.
    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|gate| gate.as_str() == name)
    }
}

#[cfg(test)]
#[path = "admission_gate_tests.rs"]
mod tests;
