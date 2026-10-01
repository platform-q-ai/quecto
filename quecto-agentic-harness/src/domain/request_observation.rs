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
    /// record stays: a retry sends the same input again.
    pub fn record_input_prefix(&self, prefix: InputPrefix) {
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
/// same session (#2398), as written: counts, indices, a kind and token
/// estimates, never content. Every estimate uses one estimator, so they
/// compare with each other and with the provider's `cache_read_tokens`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct InputPrefixParts {
    /// The input items the request sent.
    pub input_items: usize,
    /// The input items of the previous request it was compared with;
    /// `None` when there was none to compare with (the session's first
    /// observed request, or one its provider no longer remembers).
    pub previous_items: Option<usize>,
    /// The first item whose serialized bytes differ from the previous
    /// request's item at the same index (or that the previous request had
    /// and this one lacks); `None` when the previous input is a
    /// byte-identical prefix of this one (append-only), or when nothing was
    /// compared.
    pub first_changed_item: Option<usize>,
    /// That item's kind; `None` when it is not an item this request sent.
    pub first_changed_kind: Option<InputItemKind>,
    /// The estimated tokens of the unchanged input items.
    pub prefix_tokens_estimate: usize,
    /// The estimated tokens of the unchanged prefix of the whole request:
    /// its instructions and tools, when they are unchanged, and then its
    /// unchanged input items; `0` when the instructions or tools changed.
    pub unchanged_prefix_tokens_estimate: usize,
    /// The estimated tokens of the whole request: instructions, tools and
    /// every input item.
    pub request_tokens_estimate: usize,
}

/// An [`InputPrefixParts`] whose counts agree with each other (#2398).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(try_from = "InputPrefixParts", into = "InputPrefixParts")]
pub struct InputPrefix(InputPrefixParts);

/// Why an [`InputPrefixParts`] is not a consistent record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidInputPrefix(pub &'static str);

impl std::fmt::Display for InvalidInputPrefix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "inconsistent input prefix: {}", self.0)
    }
}

impl InputPrefix {
    /// `parts`, when its counts agree with each other.
    pub fn new(parts: InputPrefixParts) -> Result<Self, InvalidInputPrefix> {
        let InputPrefixParts {
            input_items,
            previous_items,
            first_changed_item: item,
            first_changed_kind: kind,
            prefix_tokens_estimate: prefix,
            unchanged_prefix_tokens_estimate: unchanged,
            request_tokens_estimate: request,
        } = parts;
        let rules = [
            (
                kind.is_none() || item.is_some(),
                "a kind names a changed item",
            ),
            (
                previous_items.is_some() || (prefix == 0 && unchanged == 0),
                "nothing compared leaves no unchanged prefix",
            ),
            (
                item.is_some() || previous_items.is_none_or(|previous| previous <= input_items),
                "an unchanged previous input is a prefix of this one",
            ),
            (
                item.is_none_or(|index| {
                    previous_items.is_some_and(|previous| index < previous) && index <= input_items
                }),
                "a changed item was compared, within the previous input and at most one past this one",
            ),
            (
                item.is_none_or(|index| index < input_items || kind.is_none()),
                "an item this request lacks has no kind",
            ),
            (
                item.is_none_or(|index| index > 0 || prefix == 0),
                "a change at the first item leaves no unchanged item",
            ),
            (
                prefix <= request && unchanged <= request,
                "a prefix is part of its request",
            ),
            (
                unchanged == 0 || unchanged >= prefix,
                "the unchanged whole prefix includes the unchanged items",
            ),
        ];
        let broken = rules.into_iter().find_map(|(holds, reason)| match holds {
            true => None,
            false => Some(reason),
        });
        match broken {
            None => Ok(Self(parts)),
            Some(reason) => Err(InvalidInputPrefix(reason)),
        }
    }

    /// The record's counts.
    pub fn parts(self) -> InputPrefixParts {
        self.0
    }
}

impl TryFrom<InputPrefixParts> for InputPrefix {
    type Error = InvalidInputPrefix;
    fn try_from(parts: InputPrefixParts) -> Result<Self, Self::Error> {
        Self::new(parts)
    }
}

impl From<InputPrefix> for InputPrefixParts {
    fn from(prefix: InputPrefix) -> Self {
        prefix.0
    }
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
