//! Request diagnostics contain measurements and availability, never prompt content or billing claims.
use super::attempt_diagnostics::AttemptDiagnostics;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Debug, Default)]
pub struct RequestTrace {
    attempts: AtomicU32,
    oauth_retries: AtomicU32,
    diagnostics: Mutex<Vec<AttemptDiagnostics>>,
}
impl RequestTrace {
    /// Retain a bounded prefix of actual transport attempts, in completion order.
    pub fn record_attempt(&self, record: AttemptDiagnostics) {
        let mut records = self.diagnostics.lock().unwrap_or_else(|e| e.into_inner());
        if records.len() < 16 {
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
    pub harness_prefix_sha256: String,
    pub harness_prefix_bytes: usize,
    pub harness_prefix_unchanged: Option<bool>,
}

pub trait RequestAccounting: Send + Sync {
    fn record<'a>(
        &'a self,
        observation: &'a RequestObservation,
    ) -> super::subagent_launch::LaunchFuture<'a, Result<(), super::error::DomainError>>;
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
