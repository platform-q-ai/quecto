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
mod tests {
    use super::*;

    #[test]
    fn maximum_produced_attempt_payload_fits_accounting_envelope() {
        use crate::domain::attempt_diagnostics::{HeaderName, HeaderValue, SafeHeader};
        let trace = RequestTrace::default();
        for attempt_number in 1..=16 {
            trace.record_attempt(AttemptDiagnostics {
                attempt_number,
                started_unix_ms: u64::MAX,
                finished_unix_ms: u64::MAX,
                elapsed_ms: u64::MAX,
                headers: (0..10)
                    .map(|_| SafeHeader {
                        name: HeaderName::XRequestId,
                        value: HeaderValue::Sha256("a".repeat(64)),
                    })
                    .collect(),
                ..Default::default()
            });
        }
        // Leave at least 4 KiB for existing observation/runtime measurements.
        assert!(
            serde_json::to_vec(&trace.attempt_diagnostics())
                .unwrap()
                .len()
                < 28_672
        );
    }

    #[test]
    fn attempt_retention_is_bounded_and_preserves_attempt_numbers() {
        let trace = RequestTrace::default();
        for attempt_number in 1..=20 {
            trace.record_attempt(AttemptDiagnostics {
                attempt_number,
                wire_status: Some(200),
                ..Default::default()
            });
        }
        let records = trace.attempt_diagnostics();
        assert_eq!(records.len(), 16);
        assert_eq!(records[0].attempt_number, 1);
        assert_eq!(records[15].attempt_number, 16);
    }
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;

    #[test]
    fn legacy_observation_has_unavailable_wire_evidence() {
        let legacy = serde_json::json!({
            "request_id":"legacy", "model":"test", "provider":"test", "outcome":"failed",
            "error_class":"server", "estimated_context_tokens":0, "instrumented_attempts":3,
            "oauth_retries":0, "duration_ms":10, "harness_prefix_sha256":"",
            "harness_prefix_bytes":0
        });
        let record: RequestObservation = serde_json::from_value(legacy).unwrap();
        assert_eq!(record.started_unix_ms, None);
        assert_eq!(record.finished_unix_ms, None);
        assert!(record.attempt_diagnostics.is_empty());
    }
}
