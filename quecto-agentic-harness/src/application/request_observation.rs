//! Captures one logical request, including cancellation when its future is dropped.
//! While it runs it is the agent's request in flight, which `get_state`
//! reports; dropped mid-attempt, it records that attempt as `Interrupted`
//! and queues itself for its audit record (#2210).
use crate::application::providers::ports::ChatRequest;
use crate::domain::conversation::value_objects::message::UsageInfo;
use crate::domain::conversation::value_objects::message::{LlmResponse, Role};
use crate::domain::error::DomainError;
use crate::domain::inference::events::request_completion::RequestOutcome;
use crate::domain::inference::events::request_observation::{
    RequestDiagnostics, RequestObservation, RequestTrace,
};
use crate::domain::inference::events::request_progress::InFlightRequest;
use crate::domain::inference::services::provider_error::classify_provider_error;
use sha2::{Digest, Sha256};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
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
    /// The agent's request in flight, which `get_state` reports.
    pub in_flight: &'a InFlightRequest,
    /// Requests that ended in flight, awaiting their audit record.
    pub interrupted: &'a Mutex<Vec<InterruptedRequest>>,
    /// The loop turn the request was sent in, for its audit record.
    pub turn: u32,
}

/// A request that ended in flight, awaiting its audit record (#2210).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct InterruptedRequest {
    /// The loop turn it was sent in.
    pub turn: u32,
    pub observation: RequestObservation,
}

pub(super) struct ObservationGuard<'a> {
    sinks: ObservationSinks<'a>,
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
        let started = Instant::now();
        sinks.in_flight.begin(started, trace.clone());
        Self {
            sinks,
            trace,
            started,
            record: Some(RequestObservation {
                request_id: uuid::Uuid::new_v4().to_string(),
                started_unix_ms: unix_ms(),
                finished_unix_ms: None,
                attempt_diagnostics: Vec::new(),
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
                first_token_ms: None,
                harness_prefix_sha256: prefix.sha256,
                harness_prefix_bytes: prefix.bytes,
                harness_prefix_unchanged: prefix.unchanged,
                ended_empty_after_tools: false,
                input_prefix: None,
            }),
        }
    }

    /// The reply had nothing in it and ended the turn (#2434): noted on the
    /// record before it is finished.
    pub fn note_ended_empty_after_tools(&mut self) {
        let record = self.record.as_mut().expect("noted before completion");
        record.ended_empty_after_tools = true;
    }

    pub fn finish(&mut self, result: &Result<LlmResponse, DomainError>) -> RequestObservation {
        let record = self.record.as_mut().expect("one completion per request");
        // What the request spent: every attempt cut short after reporting
        // usage (#2249 review), then the reply's own, each counted once —
        // as the billed totals count them.
        let mut spent = self.trace.unfinished_usage();
        // #2436: the attempt in flight ends with the request.
        let (ended, reply) = match result {
            Ok(response) => (RequestOutcome::Ok, response.usage.as_ref()),
            Err(_) => (RequestOutcome::Error, None),
        };
        self.trace.end_attempt(ended, reply);
        match result {
            Ok(response) => {
                record.outcome = "succeeded".into();
                spent.extend(response.usage.clone());
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
        observe_spend(record, &spent);
        self.publish(Ending::Finished)
            .expect("completed observation exists")
    }

    /// The request's trace.
    pub fn trace(&self) -> Arc<RequestTrace> {
        self.trace.clone()
    }

    /// Publish the record once; `ending` says whether the request finished
    /// or was dropped in flight.
    fn publish(&mut self, ending: Ending) -> Option<RequestObservation> {
        let mut record = self.record.take()?;
        if let Ending::Dropped = ending {
            // #2436: an attempt still in flight was cancelled with it.
            self.trace.end_attempt(RequestOutcome::Cancelled, None);
        }
        self.sinks.in_flight.end(&self.trace);
        record.duration_ms = self
            .started
            .elapsed()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX);
        record.finished_unix_ms = unix_ms();
        record.attempt_diagnostics = match ending {
            Ending::Finished => self.trace.attempt_diagnostics(),
            // A stream in its own task may still run, and a transport run
            // inside the request has recorded `Dropped` on its way out:
            // either way the attempt in flight reads as `Interrupted`.
            Ending::Dropped => self
                .trace
                .attempts_when_dropped(Instant::now(), record.finished_unix_ms),
        };
        record.first_token_ms = self.trace.first_token().map(|at| {
            let since = at.saturating_duration_since(self.started).as_millis();
            u64::try_from(since).unwrap_or(u64::MAX)
        });
        record.input_prefix = self.trace.input_prefix();
        record.instrumented_attempts = self.trace.attempts();
        record.oauth_retries = self.trace.oauth_retries();
        if let Some(outbox) = self.sinks.outbox {
            outbox
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(record.clone());
        }
        if let Ending::Dropped = ending {
            let mut interrupted = self
                .sinks
                .interrupted
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            // Bounded as the recent log is: a mode that never audits them
            // keeps only the newest.
            if interrupted.len() >= INTERRUPTED_RETAINED {
                interrupted.remove(0);
            }
            interrupted.push(InterruptedRequest {
                turn: self.sinks.turn,
                observation: record.clone(),
            });
        }
        self.sinks
            .log
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .record(record.clone());
        Some(record)
    }
}

/// A request's future, which marks its trace as dropping when it is dropped
/// before it completes (#2210 review) — before the future itself, and with
/// it the transports it owns, is dropped — so an attempt a transport records
/// on its way out is known to have been dropped with its request.
pub(super) struct MarkDropping<F> {
    trace: Arc<RequestTrace>,
    request: Pin<Box<F>>,
    completed: bool,
}

impl<F> MarkDropping<F> {
    pub(super) fn new(trace: Arc<RequestTrace>, request: F) -> Self {
        Self {
            trace,
            request: Box::pin(request),
            completed: false,
        }
    }
}

impl<F: Future> Future for MarkDropping<F> {
    type Output = F::Output;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        let output = self.request.as_mut().poll(cx);
        if output.is_ready() {
            self.completed = true;
        }
        output
    }
}

impl<F> Drop for MarkDropping<F> {
    fn drop(&mut self) {
        // Runs before the fields drop: the request is still whole here.
        if !self.completed {
            self.trace.mark_dropping();
        }
    }
}

/// The most requests ended in flight kept for their audit record.
const INTERRUPTED_RETAINED: usize = 64;

/// How a request's observation ended.
#[derive(Clone, Copy)]
enum Ending {
    /// The request returned its result.
    Finished,
    /// The request's future was dropped before it returned (a run deadline,
    /// an abort or steer, a shutdown).
    Dropped,
}

impl Drop for ObservationGuard<'_> {
    fn drop(&mut self) {
        self.publish(Ending::Dropped);
    }
}

fn unix_ms() -> Option<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|value| value.as_millis().try_into().ok())
}

/// Fold `spent` (each attempt's reported usage, the last the reply's own)
/// into `record`: token counts summed, a cache count only when an attempt
/// reported one, the cost only when every attempt was priced, and the
/// context occupancy the last attempt's. Nothing spent leaves the record's
/// usage unknown.
fn observe_spend(record: &mut RequestObservation, spent: &[UsageInfo]) {
    let Some(last) = spent.last() else {
        return;
    };
    let sum = |field: fn(&UsageInfo) -> u32| spent.iter().map(|u| u64::from(field(u))).sum();
    let optional = |field: fn(&UsageInfo) -> Option<u32>| {
        spent
            .iter()
            .filter_map(field)
            .map(u64::from)
            .reduce(|a, b| a.saturating_add(b))
    };
    record.input_tokens = Some(sum(|u| u.prompt_tokens));
    record.output_tokens = Some(sum(|u| u.completion_tokens));
    record.cache_read_tokens = optional(|u| u.cache_read_tokens);
    record.cache_write_tokens = optional(|u| u.cache_write_tokens);
    record.context_input_tokens = Some(u64::from(last.context_input_tokens()));
    record.estimated_cost_micro_usd = spent
        .iter()
        .map(|u| u.cost.as_ref().map(|cost| cost.total_cost_micro_usd))
        .sum();
}

#[cfg(test)]
#[path = "request_observation_guard_tests.rs"]
mod tests;
