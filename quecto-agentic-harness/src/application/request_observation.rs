//! Captures one logical request, including cancellation when its future is dropped.
use crate::domain::{
    error::DomainError,
    message::{LlmResponse, Role},
    provider::ChatRequest,
    provider_error::classify_provider_error,
    request_observation::{RequestDiagnostics, RequestObservation, RequestTrace},
};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Diagnostic of the harness system/tool prefix, not provider cache eligibility.
pub(super) fn prefix(request: &ChatRequest<'_>) -> (String, usize) {
    let system: Vec<_> = request
        .messages
        .iter()
        .filter(|message| message.role == Role::System)
        .map(|message| &message.content)
        .collect();
    let tools: Vec<_> = request
        .tools
        .iter()
        .map(|tool| (&tool.name, &tool.description, &tool.parameters_schema))
        .collect();
    let bytes =
        serde_json::to_vec(&(system, tools)).expect("string-only request prefix serializes");
    (format!("{:x}", Sha256::digest(&bytes)), bytes.len())
}

pub(super) struct PrefixObservation {
    pub sha256: String,
    pub bytes: usize,
    pub unchanged: Option<bool>,
}

pub(super) struct ObservationSinks<'a> {
    pub log: &'a Mutex<RequestDiagnostics>,
    pub outbox: Option<&'a Mutex<Vec<RequestObservation>>>,
}

pub(super) struct ObservationGuard<'a> {
    log: &'a Mutex<RequestDiagnostics>,
    outbox: Option<&'a Mutex<Vec<RequestObservation>>>,
    record: Option<RequestObservation>,
    trace: Arc<RequestTrace>,
    started: Instant,
}
impl<'a> ObservationGuard<'a> {
    pub fn new(
        sinks: ObservationSinks<'a>,
        request: &ChatRequest<'_>,
        provider: &str,
        estimate: usize,
        prefix: PrefixObservation,
        trace: Arc<RequestTrace>,
    ) -> Self {
        Self {
            log: sinks.log,
            outbox: sinks.outbox,
            trace,
            started: Instant::now(),
            record: Some(RequestObservation {
                request_id: uuid::Uuid::new_v4().to_string(),
                model: request.model.into(),
                provider: provider.into(),
                outcome: "cancelled".into(),
                error_class: None,
                input_tokens: None,
                context_input_tokens: None,
                output_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
                estimated_cost_micro_usd: None,
                estimated_context_tokens: estimate,
                instrumented_attempts: 0,
                oauth_retries: 0,
                duration_ms: 0,
                harness_prefix_sha256: prefix.sha256,
                harness_prefix_bytes: prefix.bytes,
                harness_prefix_unchanged: prefix.unchanged,
            }),
        }
    }

    pub fn finish(&mut self, result: &Result<LlmResponse, DomainError>) -> RequestObservation {
        let record = self.record.as_mut().expect("one completion per request");
        match result {
            Ok(response) => {
                record.outcome = "succeeded".into();
                if let Some(usage) = &response.usage {
                    record.input_tokens = Some(usage.prompt_tokens as u64);
                    record.context_input_tokens = Some(usage.context_input_tokens() as u64);
                    record.output_tokens = Some(usage.completion_tokens as u64);
                    record.cache_read_tokens = usage.cache_read_tokens.map(u64::from);
                    record.cache_write_tokens = usage.cache_write_tokens.map(u64::from);
                    record.estimated_cost_micro_usd =
                        usage.cost.as_ref().map(|cost| cost.total_cost_micro_usd);
                }
            }
            Err(error) => {
                record.outcome = if self.trace.attempts() == 0 {
                    "rejected"
                } else {
                    "failed"
                }
                .into();
                record.error_class = Some(classify_provider_error(error));
            }
        }
        self.publish().expect("completed observation exists")
    }

    fn publish(&mut self) -> Option<RequestObservation> {
        let mut record = self.record.take()?;
        record.duration_ms = self
            .started
            .elapsed()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX);
        record.instrumented_attempts = self.trace.attempts();
        record.oauth_retries = self.trace.oauth_retries();
        if let Some(outbox) = self.outbox {
            outbox
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(record.clone());
        }
        self.log
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .record(record.clone());
        Some(record)
    }
}
impl Drop for ObservationGuard<'_> {
    fn drop(&mut self) {
        self.publish();
    }
}
