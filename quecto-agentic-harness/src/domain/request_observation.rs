//! Request diagnostics contain measurements and availability, never prompt content or billing claims.
//!
//! The accounting port a completed observation is recorded through is the
//! application's (`application::providers::ports::RequestAccounting`, #1960).
use super::attempt_diagnostics::{AttemptDiagnostics, Termination};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

/// The most attempt records one request retains: a bounded prefix.
pub const MAX_ATTEMPT_RECORDS: usize = 16;

#[derive(Debug, Default)]
pub struct RequestTrace {
    attempts: AtomicU32,
    oauth_retries: AtomicU32,
    pub(super) diagnostics: Mutex<Vec<AttemptDiagnostics>>,
    /// When the request's first token arrived, on the monotonic clock
    /// (#2151): the earliest any attempt saw.
    first_token: Mutex<Option<std::time::Instant>>,
    /// The attempt in flight (#2210): see `request_progress`.
    pub(super) live: Mutex<super::request_progress::LiveAttempt>,
    /// Each attempt's output cap in bytes, zero for none (#2210).
    pub(super) output_cap: std::sync::atomic::AtomicU64,
    /// The request is being dropped (#2210 review): see `mark_dropping`.
    pub(super) dropping: std::sync::atomic::AtomicBool,
    /// Usage attempts reported before they were cut short (#2249 review):
    /// tokens the provider counted for a reply that never completed.
    unfinished_usage: Mutex<Vec<crate::domain::message::UsageInfo>>,
    /// How the request's input relates to its session's previous request
    /// (#2398), as its provider first serialized it.
    input_prefix: Mutex<Option<InputPrefix>>,
}
impl RequestTrace {
    /// The provider serialized the request's input (#2398). The first
    /// record stays: a retry sends the same input again, which the
    /// provider then compares with itself.
    pub fn record_input_prefix(&self, prefix: InputPrefix) {
        debug_assert!(
            prefix.first_changed_item.is_some() || prefix.first_changed_kind.is_none(),
            "a kind names a changed item"
        );
        debug_assert!(
            prefix
                .first_changed_item
                .is_none_or(|index| index <= prefix.input_items),
            "a changed item is one this request sent, or the first it lacks"
        );
        self.input_prefix
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert(prefix);
    }
    /// How the request's input relates to its session's previous request,
    /// when its provider observed it.
    pub fn input_prefix(&self) -> Option<InputPrefix> {
        *self.input_prefix.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// An attempt was cut short after its provider reported `usage` (#2249
    /// review): those tokens were spent, so they are still counted.
    pub fn record_unfinished_usage(&self, usage: crate::domain::message::UsageInfo) {
        self.unfinished_usage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(usage);
    }
    /// The usage every cut-short attempt of this request reported.
    pub fn unfinished_usage(&self) -> Vec<crate::domain::message::UsageInfo> {
        self.unfinished_usage
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    /// A token arrived at `at`; the earliest stays.
    pub fn mark_first_token(&self, at: std::time::Instant) {
        let mut first = self.first_token.lock().unwrap_or_else(|e| e.into_inner());
        *first = Some(first.map_or(at, |earlier| earlier.min(at)));
    }
    /// When the request's first token arrived, if one did.
    pub fn first_token(&self) -> Option<std::time::Instant> {
        *self.first_token.lock().unwrap_or_else(|e| e.into_inner())
    }
    /// Retain a bounded prefix of actual transport attempts, in completion
    /// order; the recorded attempt is no longer in flight.
    /// The live lock is held across the close and the push (#2210 review),
    /// so a request read while it ends sees the attempt either in flight or
    /// recorded, never neither.
    pub fn record_attempt(&self, mut record: AttemptDiagnostics) {
        let mut live = self.live();
        // A transport dropped with its request (every `chat` runs inside
        // it) records `Dropped` on its way out: that is the interruption.
        if self.dropping.load(Ordering::SeqCst) && record.termination == Termination::Dropped {
            record.termination = Termination::Interrupted;
        }
        live.close(record.attempt_number);
        let mut records = self.diagnostics.lock().unwrap_or_else(|e| e.into_inner());
        if records.len() < MAX_ATTEMPT_RECORDS {
            records.push(record);
        }
    }
    pub fn attempt_diagnostics(&self) -> Vec<AttemptDiagnostics> {
        self.diagnostics
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn start(&self) {
        self.attempts.fetch_max(1, Ordering::Relaxed);
    }
    pub fn retry(&self) {
        self.attempts.fetch_add(1, Ordering::Relaxed);
    }
    pub fn oauth_retry(&self) {
        self.oauth_retries.fetch_add(1, Ordering::Relaxed);
        self.retry();
    }
    pub fn attempts(&self) -> u32 {
        self.attempts.load(Ordering::Relaxed)
    }
    pub fn oauth_retries(&self) -> u32 {
        self.oauth_retries.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RequestObservation {
    pub request_id: String,
    #[serde(default)]
    pub started_unix_ms: Option<u64>,
    #[serde(default)]
    pub finished_unix_ms: Option<u64>,
    /// Empty means unavailable, not evidence of a healthy transport.
    #[serde(default)]
    pub attempt_diagnostics: Vec<AttemptDiagnostics>,
    pub model: String,
    pub provider: String,
    pub outcome: String,
    pub error_class: Option<super::provider_error::ProviderErrorClass>,
    pub input_tokens: Option<u64>,
    pub context_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub estimated_cost_micro_usd: Option<u64>,
    pub estimated_context_tokens: usize,
    pub instrumented_attempts: u32,
    pub oauth_retries: u32,
    pub duration_ms: u64,
    /// When the request saw its first token, from its start (#2151); `None`
    /// when no attempt saw one (or none was observed).
    #[serde(default)]
    pub first_token_ms: Option<u64>,
    pub harness_prefix_sha256: String,
    pub harness_prefix_bytes: usize,
    pub harness_prefix_unchanged: Option<bool>,
    /// Where the request's input first differs from its session's previous
    /// request (#2398); absent when its provider does not observe its input.
    #[serde(flatten)]
    pub input_prefix: Option<InputPrefix>,
}

/// The kind of one item of a request's input (#2398).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InputItemKind {
    User,
    Assistant,
    FunctionCall,
    FunctionCallOutput,
    Reasoning,
}

/// How a request's serialized input relates to the previous request of the
/// same session (#2398): counts, an index and a kind, never content.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputPrefix {
    /// The input items the request sent.
    pub input_items: usize,
    /// The first item whose serialized bytes differ from the previous
    /// request's item at the same index (or that the previous request had
    /// and this one lacks); `None` when the previous input is a
    /// byte-identical prefix of this one (append-only, and so the first
    /// request of a session).
    pub first_changed_item: Option<usize>,
    /// That item's kind; `None` when it is not an item this request sent.
    pub first_changed_kind: Option<InputItemKind>,
    /// The estimated tokens of the unchanged prefix.
    pub prefix_tokens_estimate: usize,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RequestDiagnostics {
    pub logical_requests: u64,
    pub instrumented_attempts: u64,
    pub oauth_retries: u64,
    pub unavailable_usage_requests: u64,
    pub recent: Vec<RequestObservation>,
}
impl RequestDiagnostics {
    pub fn record(&mut self, record: RequestObservation) {
        self.logical_requests = self.logical_requests.saturating_add(1);
        self.instrumented_attempts = self
            .instrumented_attempts
            .saturating_add(record.instrumented_attempts as u64);
        self.oauth_retries = self
            .oauth_retries
            .saturating_add(record.oauth_retries as u64);
        if record.input_tokens.is_none() {
            self.unavailable_usage_requests = self.unavailable_usage_requests.saturating_add(1);
        }
        if self.recent.len() == 64 {
            self.recent.remove(0);
        }
        self.recent.push(record);
    }
    pub fn merge(&mut self, mut incoming: Self) {
        self.logical_requests = self
            .logical_requests
            .saturating_add(incoming.logical_requests);
        self.instrumented_attempts = self
            .instrumented_attempts
            .saturating_add(incoming.instrumented_attempts);
        self.oauth_retries = self.oauth_retries.saturating_add(incoming.oauth_retries);
        self.unavailable_usage_requests = self
            .unavailable_usage_requests
            .saturating_add(incoming.unavailable_usage_requests);
        self.recent.append(&mut incoming.recent);
        let excess = self.recent.len().saturating_sub(64);
        self.recent.drain(..excess);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeIdentity {
    pub process_instance_id: String,
    pub executable_digest_pending: bool,
    pub package_version: String,
    pub build_source_revision: Option<String>,
    pub build_dirty: Option<bool>,
    pub executable_sha256: Option<String>,
}

#[cfg(test)]
#[path = "request_observation_tests.rs"]
mod tests;
