//! Optional leaf transport ownership. Disabled adapters never enter this module.
//! The owned future is destroyed before its permit is acknowledged, including
//! task abortion, deadline expiry and receiver closure during a bounded send.
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use super::admission_feedback::{CooldownHint, is_typed_throttle, normalize_throttle};
use super::attempt_profile::{Profile, Vendor};
use super::sse_common::{SseHandler, SseLineOutcome};
use crate::application::ports::{AttemptAdmission, AttemptPermit};
use crate::domain::error::DomainError;
use crate::domain::inference_admission::{Feedback, ThrottleFeedback};
use crate::domain::message::LlmResponse;
use crate::domain::provider::{CancelFlag, StreamEvent};

#[path = "attempt_events.rs"]
mod attempt_events;
#[path = "attempt_output.rs"]
mod attempt_output;
#[path = "attempt_observer.rs"]
mod observer;
use observer::{LineObserver, ProtocolObserver};
#[path = "diagnostic_sse.rs"]
mod diagnostic_sse;
#[path = "transport_diagnostics.rs"]
mod diagnostics;
use crate::domain::attempt_diagnostics::*;
use crate::domain::request_observation::RequestTrace;

type Sender = tokio::sync::mpsc::Sender<StreamEvent>;
#[derive(Clone)]
pub(super) struct Receipt(Arc<Mutex<State>>);
struct State {
    permit: Option<Box<dyn AttemptPermit>>,
    failure: bool,
    hinted: bool,
    trace: Option<Arc<RequestTrace>>,
    diagnostics: AttemptDiagnostics,
    started: std::time::Instant,
}
impl Receipt {
    /// A new attempt's receipt: the permit its gate granted (none for an
    /// attempt observed without admission, #2151) and its request's trace.
    fn new(permit: Option<Box<dyn AttemptPermit>>, trace: Option<Arc<RequestTrace>>) -> Self {
        let attempt_number = trace.as_ref().map_or(1, |t| t.attempts().max(1));
        let (started, started_unix_ms) = (std::time::Instant::now(), diagnostics::unix_ms());
        // The trace follows the attempt live until its record (#2210).
        if let Some(trace) = &trace {
            trace.begin_attempt(attempt_number, started, started_unix_ms);
        }
        Receipt(Arc::new(Mutex::new(State {
            permit,
            failure: false,
            hinted: false,
            diagnostics: AttemptDiagnostics {
                attempt_number,
                started_unix_ms,
                ..Default::default()
            },
            trace,
            started,
        })))
    }

    /// The attempt's output passed its request's output cap (#2210): the
    /// error to end it with, or `None` while it is within the cap (or its
    /// request has none).
    fn capped(&self) -> Option<crate::domain::request_progress::OutputCapped> {
        let state = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let cap = state.trace.as_ref()?.output_cap()?;
        (state.diagnostics.output_bytes > cap)
            .then_some(crate::domain::request_progress::OutputCapped { cap })
    }

    /// Record the attempt into its request's trace: once, when it ends.
    fn record(state: &mut State) {
        state.diagnostics.finished_unix_ms = diagnostics::unix_ms();
        state.diagnostics.elapsed_ms =
            state.started.elapsed().as_millis().min(u64::MAX as u128) as u64;
        debug_assert!(
            state
                .diagnostics
                .first_token_ms
                .is_none_or(|first| first <= state.diagnostics.elapsed_ms),
            "a first token arrives within its attempt"
        );
        if let Some(trace) = &state.trace {
            trace.record_attempt(state.diagnostics.clone());
        }
    }

    fn headers(&self, response: &reqwest::Response) {
        let mut state = self.0.lock().unwrap();
        state.diagnostics.wire_status = Some(response.status().as_u16());
        state.diagnostics.headers = diagnostics::headers(
            response
                .headers()
                .iter()
                .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.as_str(), v))),
        );
        // Throttle feedback goes to admission; an attempt no gate started
        // (#2151) has none to report to.
        let state = &mut *state;
        let Some(permit) = state.permit.as_mut() else {
            return;
        };
        let (mono, wall) = permit.receipt_clock();
        let hint = normalize_throttle(
            response.status().as_u16(),
            response
                .headers()
                .iter()
                .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.as_str(), v))),
            wall,
            mono,
            permit.maximum_cooldown_ms(),
        );
        match hint {
            Some(CooldownHint::Until(until)) => {
                permit.feedback(ThrottleFeedback::Until(until));
                state.hinted = true;
            }
            Some(CooldownHint::Unavailable) => {
                permit.feedback(ThrottleFeedback::Unavailable);
                state.hinted = true;
            }
            _ => {}
        }
    }
    fn typed(&self, value: &serde_json::Value) {
        self.typed_as(value, is_typed_throttle(value));
    }
    /// A typed error payload that is (`throttled`) or is not a throttle.
    fn typed_as(&self, value: &serde_json::Value, throttled: bool) {
        debug_assert!(
            !is_typed_throttle(value) || throttled,
            "a typed throttle is always a throttle"
        );
        let mut state = self.0.lock().unwrap();
        diagnostics::typed(&mut state.diagnostics, value);
        if throttled {
            state.failure = true;
        }
        let state = &mut *state;
        if let (false, true, Some(permit)) = (state.hinted, throttled, state.permit.as_mut()) {
            permit.throttle_without_hint();
            state.hinted = true;
        }
    }
    fn http_error(&self, status: u16, body: &str) {
        let value = serde_json::from_str::<serde_json::Value>(body).unwrap_or_default();
        let terminal = [&value["error"], &value["response"]["error"]]
            .iter()
            .any(|error| {
                ["type", "code"].iter().any(|field| {
                    error[*field].as_str().is_some_and(|name| {
                        crate::domain::provider_error::is_billing_error_name(name)
                            || matches!(
                                name,
                                "authentication_error"
                                    | "permission_error"
                                    | "invalid_request_error"
                                    | "invalid_api_key"
                            )
                    })
                })
            });
        let mut state = self.0.lock().unwrap();
        diagnostics::typed(&mut state.diagnostics, &value);
        state.failure = true;
        let throttled = matches!(status, 429 | 529) || is_typed_throttle(&value);
        let state = &mut *state;
        if let (false, false, true, Some(permit)) =
            (state.hinted, terminal, throttled, state.permit.as_mut())
        {
            permit.throttle_without_hint();
            state.hinted = true;
        }
    }
    fn termination(&self, termination: Termination) {
        self.0.lock().unwrap().diagnostics.termination = termination;
    }
    fn fail(&self) {
        self.0.lock().unwrap().failure = true;
    }
}

/// Field destruction is explicit: no permit release on merely dropping a receiver.
struct OwnedTransport<F> {
    operation: Option<Pin<Box<F>>>,
    receipt: Receipt,
    completed: bool,
}
impl<F: Future> Future for OwnedTransport<F> {
    type Output = F::Output;
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let result = self.operation.as_mut().unwrap().as_mut().poll(cx);
        if result.is_ready() {
            self.completed = true;
        }
        result
    }
}
impl<F> Drop for OwnedTransport<F> {
    fn drop(&mut self) {
        drop(self.operation.take());
        let mut state = self.receipt.0.lock().unwrap();
        let feedback = if self.completed && !state.failure {
            Feedback::Success
        } else {
            Feedback::Failure
        };
        Receipt::record(&mut state);
        if let Some(permit) = state.permit.take() {
            permit.finish(feedback);
        }
    }
}
async fn cancelled(flag: Option<&CancelFlag>) {
    let Some(flag) = flag else {
        return std::future::pending().await;
    };
    // Woken by the cancel itself (#2155): no timer, no polling.
    let mut watch = flag.watch();
    std::future::poll_fn(|cx| match watch.cancelled_or_wake(cx.waker()) {
        true => Poll::Ready(()),
        false => Poll::Pending,
    })
    .await
}
async fn closed(tx: Option<&Sender>) {
    match tx {
        Some(tx) => tx.closed().await,
        None => std::future::pending().await,
    }
}
enum AttemptError {
    Stopped,
    Failed(DomainError),
}
impl AttemptError {
    fn into_domain(self) -> DomainError {
        match self {
            Self::Stopped => {
                DomainError::Provider("request cancelled or attempt deadline expired".into())
            }
            Self::Failed(error) => error,
        }
    }
}

async fn run<T, F: Future<Output = Result<T, DomainError>>>(
    gate: &Arc<dyn AttemptAdmission>,
    trace: Option<Arc<RequestTrace>>,
    cancel: Option<&CancelFlag>,
    tx: Option<&Sender>,
    operation: impl FnOnce(Receipt) -> F,
) -> Result<T, AttemptError> {
    let permit = tokio::select! {
        biased;
        _ = cancelled(cancel) => return Err(AttemptError::Stopped),
        _ = closed(tx) => return Err(AttemptError::Stopped),
        permit = gate.acquire() => permit.map_err(AttemptError::Failed)?,
    };
    let deadline = permit.deadline_expired();
    let receipt = Receipt::new(Some(permit), trace);
    let observed = receipt.clone();
    let operation = async move {
        let result = operation(observed.clone()).await;
        if result.is_err() {
            observed.fail();
        }
        result
    };
    let stopped = receipt.clone();
    let owned = OwnedTransport {
        operation: Some(Box::pin(operation)),
        receipt,
        completed: false,
    };
    tokio::pin!(owned);
    tokio::select! {
        biased;
        _ = cancelled(cancel) => { stopped.termination(Termination::Cancelled); Err(AttemptError::Stopped) },
        _ = closed(tx) => { stopped.termination(Termination::ReceiverClosed); Err(AttemptError::Stopped) },
        _ = deadline => { stopped.termination(Termination::Deadline); Err(AttemptError::Stopped) },
        result = &mut owned => result.map_err(AttemptError::Failed),
    }
}

pub(super) async fn text<T>(
    gate: &Arc<dyn AttemptAdmission>,
    trace: Option<Arc<RequestTrace>>,
    cancel: Option<&CancelFlag>,
    builder: reqwest::RequestBuilder,
    profile: Profile,
    parse: impl FnOnce(&str) -> Result<T, DomainError>,
) -> Result<T, DomainError> {
    run(gate, trace, cancel, None, |receipt| async move {
        // A whole reply sends nothing until complete: bounded in total.
        let read = async {
            let response = diagnostic_sse::send(builder, &receipt, profile).await?;
            if response.status().as_u16() != 200 {
                return Err(diagnostic_sse::error_body(response, &receipt, profile).await);
            }
            response.text().await.map_err(|e| {
                receipt.termination(Termination::ReadError);
                DomainError::Provider(format!("failed to read response: {e}"))
            })
        };
        let body = match profile.idle.whole(read).await {
            Ok(body) => body?,
            Err(late) => return Err(receipt.timed_out(late)),
        };
        receipt.accepted(parse(&body))
    })
    .await
    .map_err(AttemptError::into_domain)
}

/// Preserve assembled adapters' full-body parse/read semantics while observing
/// complete structured SSE lines at receipt, not after the HTTP body ends.
pub(super) async fn assembled<T>(
    gate: &Arc<dyn AttemptAdmission>,
    trace: Option<Arc<RequestTrace>>,
    cancel: Option<&CancelFlag>,
    builder: reqwest::RequestBuilder,
    profile: Profile,
    parse: impl FnOnce(&str) -> Result<T, DomainError>,
) -> Result<T, DomainError> {
    run(gate, trace, cancel, None, |receipt| async move {
        let mut response = diagnostic_sse::send(builder, &receipt, profile).await?;
        if response.status().as_u16() != 200 {
            return Err(diagnostic_sse::error_body(response, &receipt, profile).await);
        }
        let mut body = Vec::new();
        let mut observer = LineObserver::new(profile);
        let mut idle = profile.event_idle();
        loop {
            let bytes = match idle.next(response.chunk()).await {
                Ok(Ok(Some(bytes))) => bytes,
                Ok(Ok(None)) => break,
                Ok(Err(error)) => {
                    receipt.termination(Termination::ReadError);
                    return Err(profile.read_error(&error));
                }
                Err(silent) => return Err(receipt.idle(silent)),
            };
            observer.push(&bytes, &receipt);
            if let Some(capped) = receipt.capped() {
                return Err(DomainError::Provider(receipt.output_capped(capped)));
            }
            body.extend_from_slice(&bytes);
        }
        // The last line may end without a newline: observed only now, and
        // parsed with the body, so it counts against the cap (#2210 review).
        observer.finish(&receipt);
        if let Some(capped) = receipt.capped() {
            return Err(DomainError::Provider(receipt.output_capped(capped)));
        }
        receipt.accepted(parse(&String::from_utf8_lossy(&body)))
    })
    .await
    .map_err(AttemptError::into_domain)
}
/// An admitted attempt's SSE handler: every line is observed, then handled
/// by `inner` unchanged. The inner handler owns the protocol, a mid-stream
/// error chunk included (`OpenAiSseHandler` ends the stream at one, #2236),
/// so a line means the same with admission and without.
struct ObservedHandler<H> {
    inner: H,
    receipt: Receipt,
    protocol: ProtocolObserver,
}

async fn forward(
    mut rx: tokio::sync::mpsc::Receiver<StreamEvent>,
    tx: &Sender,
    receipt: &Receipt,
) -> Option<StreamEvent> {
    let mut terminal = None;
    while let Some(event) = rx.recv().await {
        if terminal.is_none() {
            if let StreamEvent::Done(response) = &event {
                let mut state = receipt.0.lock().unwrap();
                state.diagnostics.stop_reason = response.stop_reason.as_ref().map(|reason| {
                    use crate::domain::message::StopReason;
                    match reason {
                        StopReason::EndTurn => TerminalStopReason::EndTurn,
                        StopReason::MaxTokens => TerminalStopReason::MaxTokens,
                        StopReason::ToolUse => TerminalStopReason::ToolUse,
                        StopReason::Refusal => TerminalStopReason::Refusal,
                        StopReason::Error => TerminalStopReason::Error,
                        StopReason::Aborted => TerminalStopReason::Aborted,
                        StopReason::Unknown(_) => TerminalStopReason::Unknown,
                    }
                });
                state.diagnostics.generated_text |=
                    response.content.as_ref().is_some_and(|s| !s.is_empty());
                state.diagnostics.generated_tool_call |= !response.tool_calls.is_empty();
                state.diagnostics.generated_thinking |=
                    crate::domain::visible_thinking::has_visible_thinking(
                        &response.thinking_blocks,
                    );
            }
            if matches!(event, StreamEvent::Error(_)) {
                receipt.fail();
            }
            if matches!(event, StreamEvent::Done(_) | StreamEvent::Error(_)) {
                terminal = Some(event);
            } else if tx.send(event).await.is_err() {
                receipt.termination(Termination::ReceiverClosed);
                return None;
            }
        }
    }
    terminal
}
impl<H: SseHandler> SseHandler for ObservedHandler<H> {
    async fn process_line(&mut self, line: &str, tx: &Sender) -> SseLineOutcome {
        self.protocol.observe(line, &self.receipt);
        let outcome = self.inner.process_line(line, tx).await;
        if matches!(outcome, SseLineOutcome::Done) {
            self.receipt.refused();
        }
        outcome
    }
    async fn on_eof(&mut self, tx: &Sender) {
        end_at_eof(&mut self.inner, tx, &self.receipt).await;
    }
}

/// End a stream at end of file through `inner`, recording how the attempt
/// ended from what `inner` sent (#2249 review): a reply (an OpenAI reply
/// that named its `finish_reason`) completed it; an error means the body
/// ended before its terminal event, cut short, and fails the attempt.
async fn end_at_eof<H: SseHandler>(inner: &mut H, tx: &Sender, receipt: &Receipt) {
    let (ending, mut sent) = tokio::sync::mpsc::channel(1);
    let finish = async move {
        inner.on_eof(&ending).await;
    };
    let relay = async {
        while let Some(event) = sent.recv().await {
            match &event {
                StreamEvent::Done(_) => receipt.termination(Termination::Completed),
                StreamEvent::Error(_) => {
                    receipt.fail();
                    receipt.termination(Termination::CutShort);
                }
                _ => {}
            }
            if tx.send(event).await.is_err() {
                return;
            }
        }
    };
    tokio::join!(finish, relay);
}

pub(super) async fn stream<H: SseHandler>(
    gate: &Arc<dyn AttemptAdmission>,
    observation: (Option<Arc<RequestTrace>>, Option<&CancelFlag>),
    builder: reqwest::RequestBuilder,
    profile: Profile,
    tx: Sender,
    handler: H,
) {
    let (trace, cancel) = observation;
    let output = tx.clone();
    let result = run(gate, trace, cancel, Some(&tx), |receipt| async move {
        let mut response = diagnostic_sse::send(builder, &receipt, profile).await?;
        if response.status().as_u16() != 200 {
            return Err(diagnostic_sse::error_body(response, &receipt, profile).await);
        }
        let mut handler = ObservedHandler {
            inner: handler,
            receipt: receipt.clone(),
            protocol: ProtocolObserver::new(profile),
        };
        let (local_tx, rx) = tokio::sync::mpsc::channel(1);
        let pump = async {
            let idle = profile.idle;
            diagnostic_sse::pump_sse(&receipt, &mut response, &local_tx, &mut handler, idle).await;
            drop(local_tx);
        };
        let (_, terminal) = tokio::join!(pump, forward(rx, &output, &receipt));
        Ok(terminal)
    })
    .await;
    // The transport and permit are gone before forwarding setup/HTTP failures.
    // Natural failures retain their terminal event even under backpressure;
    // cancellation/deadline remains nonblocking after local destruction.
    if let Ok(Some(event)) = result {
        tokio::select! {
            _ = cancelled(cancel) => {},
            _ = tx.closed() => {},
            _ = tx.send(event) => {},
        }
    } else if let Err(error) = result {
        let stopped = matches!(error, AttemptError::Stopped);
        let message = match error.into_domain() {
            DomainError::Provider(message) => message,
            other => other.to_string(),
        };
        if stopped {
            let _ = tx.try_send(StreamEvent::Error(message));
        } else {
            tokio::select! {
                _ = cancelled(cancel) => {},
                _ = tx.closed() => {},
                _ = tx.send(StreamEvent::Error(message)) => {},
            }
        }
    }
}

pub(super) async fn collect(
    mut rx: tokio::sync::mpsc::Receiver<StreamEvent>,
) -> Result<LlmResponse, DomainError> {
    let mut result = None;
    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::Done(response) => result = Some(Ok(response)),
            StreamEvent::Error(error) => result = Some(Err(DomainError::Provider(error))),
            _ => {}
        }
    }
    result.unwrap_or_else(|| {
        Err(DomainError::Provider(
            "stream ended without completion".into(),
        ))
    })
}

#[cfg(test)]
#[path = "attempt_transport_tests.rs"]
mod tests;

#[path = "attempt_transport_passive.rs"]
mod passive;
pub(super) use passive::{PassiveAttempt, pump_observed};
