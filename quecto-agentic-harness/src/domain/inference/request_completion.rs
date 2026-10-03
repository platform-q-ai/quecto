//! One agent's own LLM requests (#2436).
//!
//! `admission.counters` count every admission attempt made through the
//! process's admission binding, so a supervisor could not tell one agent's
//! requests from another's. Each provider request — an attempt: a retry is
//! a request of its own — is therefore reported once when it ends, by the
//! agent that sent it: what it spent as its provider reported it, how long
//! it took, how it ended, and its number in the agent's own sequence. The
//! agent keeps running totals of the same requests ([`RequestTally`]).
//!
//! Measurements only: never prompt or output content.
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::domain::message::UsageInfo;
use crate::domain::request_observation::RequestTrace;

/// How one provider request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequestOutcome {
    /// It returned a reply.
    Ok,
    /// It failed: a provider or transport error. A retry is the next request.
    Error,
    /// It was dropped while it ran: an abort, a steer, a run deadline, a
    /// shutdown.
    Cancelled,
}

impl RequestOutcome {
    /// The outcome as the wire names it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
        }
    }
}

/// What one request spent, as its provider reported it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestSpend {
    /// Input tokens at the full price: prompt-cache reads are not included.
    pub input_tokens: u64,
    /// Input tokens served from the provider's prompt cache; `None` when the
    /// provider did not say.
    pub cached_tokens: Option<u64>,
    /// Output tokens.
    pub output_tokens: u64,
}

impl RequestSpend {
    /// The sum of `reports`, each counted once; `None` when there are none:
    /// a provider that reported nothing spent nothing we know of.
    pub fn of<'a>(reports: impl IntoIterator<Item = &'a UsageInfo>) -> Option<Self> {
        let _ = reports;
        None
    }
}

/// One provider request that ended, as its logical request's trace saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndedAttempt {
    /// Its number within its logical request, from 1: 2 and above are
    /// retries.
    pub attempt: u32,
    pub outcome: RequestOutcome,
    /// From when it started to when it ended.
    pub duration_ms: u64,
    /// `None` when its provider reported no usage for it.
    pub spend: Option<RequestSpend>,
}

/// One ended provider request, numbered in its agent's own sequence: the
/// `request_completed` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestCompleted {
    /// The model id the provider was sent.
    pub model: String,
    /// The provider the request went to.
    pub provider: String,
    /// `None` when the provider reported no usage for it.
    pub spend: Option<RequestSpend>,
    pub duration_ms: u64,
    pub outcome: RequestOutcome,
    /// This agent's requests so far, this one included: 1, 2, 3, …
    pub request_index: u64,
    /// Its number within its logical request, from 1.
    pub attempt: u32,
}

/// This agent's own LLM requests so far and what they spent, as `get_state`
/// reports them (`agentRequests`): not the admission binding's counters,
/// which every agent of the process shares. A token count sums what the
/// providers reported; a request that reported nothing adds nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentRequestCounters {
    /// Provider requests ended, retries included: the last `requestIndex`.
    pub requests: u64,
    pub input_tokens: u64,
    pub cached_tokens: u64,
    pub output_tokens: u64,
}

/// One agent's running count of its requests: the agent loop keeps one for
/// its whole life, and `get_state` reads it.
#[derive(Debug, Default)]
pub struct RequestTally(Mutex<AgentRequestCounters>);

impl RequestTally {
    /// Count `ended`, a request to `provider` for `model`: its record,
    /// numbered next in this agent's sequence.
    pub fn record(&self, model: &str, provider: &str, ended: EndedAttempt) -> RequestCompleted {
        RequestCompleted {
            model: model.into(),
            provider: provider.into(),
            spend: ended.spend,
            duration_ms: ended.duration_ms,
            outcome: ended.outcome,
            request_index: 0,
            attempt: ended.attempt,
        }
    }

    /// The counts so far.
    pub fn counters(&self) -> AgentRequestCounters {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Receives each attempt of a request as it ends. Called synchronously, on
/// whichever task ends the attempt: it must not block.
pub type AttemptEndSink = Arc<dyn Fn(EndedAttempt) + Send + Sync>;

/// Where a request's attempts are reported: the first sink attached.
#[derive(Default)]
pub(in crate::domain) struct AttemptEndHook(std::sync::OnceLock<AttemptEndSink>);

impl std::fmt::Debug for AttemptEndHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttemptEndHook")
            .field("attached", &self.0.get().is_some())
            .finish()
    }
}

/// A request's attempt in flight: started and not yet ended.
#[derive(Debug, Default)]
pub(in crate::domain) struct AttemptClock;

impl RequestTrace {
    /// Report each attempt of this request to `sink` as it ends; the first
    /// sink attached stays.
    pub fn on_attempt_end(&self, sink: AttemptEndSink) {
        let _ = self.attempt_end.0.set(sink);
    }

    /// Attempt `number` started.
    pub(in crate::domain) fn open_attempt(&self, number: u32) {
        let _ = (number, &self.attempt_clock);
    }

    /// The attempt in flight failed: it ends now, as an error.
    pub fn end_attempt_failed(&self) {
        self.end_attempt(RequestOutcome::Error, None);
    }

    /// The attempt in flight ended with `outcome`; `reply` is the usage its
    /// reply reported.
    pub fn end_attempt(&self, outcome: RequestOutcome, reply: Option<&UsageInfo>) {
        let _ = (outcome, reply);
    }
}

#[cfg(test)]
#[path = "request_completion_tests.rs"]
mod tests;
