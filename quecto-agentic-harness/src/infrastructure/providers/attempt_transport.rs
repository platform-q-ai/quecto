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

type Sender = tokio::sync::mpsc::Sender<StreamEvent>;
#[derive(Clone)]
pub(super) struct Receipt(Arc<Mutex<State>>);
struct State {
    permit: Option<Box<dyn AttemptPermit>>,
    failure: bool,
    hinted: bool,
}
impl Receipt {
    fn headers(&self, response: &reqwest::Response) {
        let mut state = self.0.lock().unwrap();
        let permit = state.permit.as_mut().unwrap();
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
        let mut state = self.0.lock().unwrap();
        if is_typed_throttle(value) {
            state.failure = true;
        }
        if !state.hinted && is_typed_throttle(value) {
            state.permit.as_mut().unwrap().throttle_without_hint();
            state.hinted = true;
        }
    }
    fn http_error(&self, status: u16, body: &str) {
        let value = serde_json::from_str::<serde_json::Value>(body).unwrap_or_default();
        let terminal = [&value["error"], &value["response"]["error"]]
            .iter()
            .any(|error| {
                ["type", "code"].iter().any(|field| {
                    matches!(
                        error[*field].as_str(),
                        Some(
                            "insufficient_quota"
                                | "billing_hard_limit_reached"
                                | "authentication_error"
                                | "permission_error"
                                | "invalid_request_error"
                                | "invalid_api_key"
                        )
                    )
                })
            });
        let mut state = self.0.lock().unwrap();
        state.failure = true;
        if !state.hinted && !terminal && (matches!(status, 429 | 529) || is_typed_throttle(&value))
        {
            state.permit.as_mut().unwrap().throttle_without_hint();
            state.hinted = true;
        }
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
        if let Some(permit) = state.permit.take() {
            permit.finish(feedback);
        }
    }
}
async fn cancelled(flag: Option<&CancelFlag>) {
    let Some(flag) = flag else {
        return std::future::pending().await;
    };
    // CancelFlag's inward API is an atomic observation, not a notification port.
    while !flag.is_cancelled() {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
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
    let receipt = Receipt(Arc::new(Mutex::new(State {
        permit: Some(permit),
        failure: false,
        hinted: false,
    })));
    let observed = receipt.clone();
    let operation = async move {
        let result = operation(observed.clone()).await;
        if result.is_err() {
            observed.fail();
        }
        result
    };
    let owned = OwnedTransport {
        operation: Some(Box::pin(operation)),
        receipt,
        completed: false,
    };
    tokio::select! {
        biased;
        _ = cancelled(cancel) => Err(AttemptError::Stopped),
        _ = closed(tx) => Err(AttemptError::Stopped),
        _ = deadline => Err(AttemptError::Stopped),
        result = owned => result.map_err(AttemptError::Failed),
    }
}

async fn send(
    builder: reqwest::RequestBuilder,
    receipt: &Receipt,
    profile: Profile,
) -> Result<reqwest::Response, DomainError> {
    let response = builder
        .send()
        .await
        .map_err(|error| profile.send_error(&error))?;
    receipt.headers(&response);
    Ok(response)
}
async fn error_body(
    response: reqwest::Response,
    receipt: &Receipt,
    profile: Profile,
) -> DomainError {
    let status = response.status().as_u16();
    let suffix = profile.suffix(response.headers());
    let text = match response.text().await {
        Ok(text) => text,
        Err(error) if profile.strict_error_body() => return profile.read_error(&error),
        Err(_) => String::new(),
    };
    receipt.http_error(status, &text);
    let text = profile.error_body(text);
    DomainError::Provider(format!(
        "HTTP {status} from {}: {text}{suffix}",
        profile.name()
    ))
}

pub(super) async fn text<T>(
    gate: &Arc<dyn AttemptAdmission>,
    cancel: Option<&CancelFlag>,
    builder: reqwest::RequestBuilder,
    profile: Profile,
    parse: impl FnOnce(&str) -> Result<T, DomainError>,
) -> Result<T, DomainError> {
    run(gate, cancel, None, |receipt| async move {
        let response = send(builder, &receipt, profile).await?;
        if response.status().as_u16() != 200 {
            return Err(error_body(response, &receipt, profile).await);
        }
        let body = response
            .text()
            .await
            .map_err(|e| DomainError::Provider(format!("failed to read response: {e}")))?;
        parse(&body)
    })
    .await
    .map_err(AttemptError::into_domain)
}

/// Preserve assembled adapters' full-body parse/read semantics while observing
/// complete structured SSE lines at receipt, not after the HTTP body ends.
pub(super) async fn assembled<T>(
    gate: &Arc<dyn AttemptAdmission>,
    cancel: Option<&CancelFlag>,
    builder: reqwest::RequestBuilder,
    profile: Profile,
    parse: impl FnOnce(&str) -> Result<T, DomainError>,
) -> Result<T, DomainError> {
    run(gate, cancel, None, |receipt| async move {
        let mut response = send(builder, &receipt, profile).await?;
        if response.status().as_u16() != 200 {
            return Err(error_body(response, &receipt, profile).await);
        }
        let mut body = Vec::new();
        let mut observer = LineObserver {
            protocol: ProtocolObserver::new(profile),
            carry: Vec::new(),
            oversized: false,
        };
        while let Some(bytes) = response
            .chunk()
            .await
            .map_err(|error| profile.read_error(&error))?
        {
            observer.push(&bytes, &receipt);
            body.extend_from_slice(&bytes);
        }
        observer.finish(&receipt);
        parse(&String::from_utf8_lossy(&body))
    })
    .await
    .map_err(AttemptError::into_domain)
}
struct LineObserver {
    carry: Vec<u8>,
    oversized: bool,
    protocol: ProtocolObserver,
}
impl LineObserver {
    fn push(&mut self, bytes: &[u8], receipt: &Receipt) {
        for byte in bytes {
            if *byte == b'\n' {
                self.finish(receipt);
            } else if !self.oversized {
                if self.carry.len() < super::sse_common::MAX_SSE_LINE_BYTES {
                    self.carry.push(*byte);
                } else {
                    self.carry.clear();
                    self.oversized = true;
                }
            }
        }
    }
    fn finish(&mut self, receipt: &Receipt) {
        if !self.oversized {
            if let Ok(line) = std::str::from_utf8(&self.carry) {
                self.protocol.observe(line.trim(), receipt);
            }
        }
        self.carry.clear();
        self.oversized = false;
    }
}

struct ProtocolObserver {
    vendor: Vendor,
    terminal: bool,
    event: String,
}
impl ProtocolObserver {
    fn new(profile: Profile) -> Self {
        Self {
            vendor: profile.vendor,
            terminal: false,
            event: String::new(),
        }
    }
    fn observe(&mut self, line: &str, receipt: &Receipt) {
        if self.terminal {
            return;
        }
        if matches!(self.vendor, Vendor::Anthropic) {
            if let Some(event) = line.strip_prefix("event: ") {
                self.event = event.to_owned();
                return;
            }
        }
        let Some(data) = line.strip_prefix("data: ") else {
            return;
        };
        if !matches!(self.vendor, Vendor::Anthropic) && data == "[DONE]" {
            self.terminal = true;
            return;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
            return;
        };
        if matches!(self.vendor, Vendor::Anthropic) {
            match self.event.as_str() {
                "error" => {
                    receipt.typed(&value);
                    receipt.fail();
                    self.terminal = true;
                }
                "message_stop" => self.terminal = true,
                _ => {}
            }
        } else {
            let (failed, completed) = match self.vendor {
                Vendor::OpenAi => (
                    value.get("error").is_some_and(serde_json::Value::is_object),
                    false,
                ),
                Vendor::Codex => (
                    matches!(
                        value["type"].as_str(),
                        Some("response.failed" | "response.incomplete" | "error")
                    ),
                    value["type"].as_str() == Some("response.completed"),
                ),
                Vendor::Anthropic => unreachable!("Anthropic dispatch handled above"),
            };
            if failed {
                receipt.typed(&value);
                receipt.fail();
            }
            self.terminal = failed || completed;
        }
    }
}

struct ObservedHandler<H> {
    inner: H,
    receipt: Receipt,
    openai_errors: bool,
    protocol: ProtocolObserver,
}

async fn forward(mut rx: tokio::sync::mpsc::Receiver<StreamEvent>, tx: &Sender, receipt: &Receipt) {
    while let Some(event) = rx.recv().await {
        if matches!(event, StreamEvent::Error(_)) {
            receipt.fail();
        }
        if tx.send(event).await.is_err() {
            return;
        }
    }
}
impl<H: SseHandler> SseHandler for ObservedHandler<H> {
    async fn process_line(&mut self, line: &str, tx: &Sender) -> SseLineOutcome {
        self.protocol.observe(line, &self.receipt);
        if self.openai_errors {
            if let Some(data) = line.strip_prefix("data: ") {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(data) {
                    if value.get("error").is_some_and(serde_json::Value::is_object) {
                        let _ = tx
                            .send(StreamEvent::Error(
                                super::attempt_profile::openai_stream_error(&value),
                            ))
                            .await;
                        return SseLineOutcome::Done;
                    }
                }
            }
        }
        self.inner.process_line(line, tx).await
    }
    async fn on_eof(&mut self, tx: &Sender) {
        self.inner.on_eof(tx).await;
    }
}

pub(super) async fn stream<H: SseHandler>(
    gate: &Arc<dyn AttemptAdmission>,
    cancel: Option<&CancelFlag>,
    builder: reqwest::RequestBuilder,
    profile: Profile,
    tx: Sender,
    handler: H,
) {
    let output = tx.clone();
    let result = run(gate, cancel, Some(&tx), |receipt| async move {
        let mut response = send(builder, &receipt, profile).await?;
        if response.status().as_u16() != 200 {
            return Err(error_body(response, &receipt, profile).await);
        }
        let mut handler = ObservedHandler {
            inner: handler,
            receipt: receipt.clone(),
            openai_errors: profile.openai(),
            protocol: ProtocolObserver::new(profile),
        };
        let (local_tx, rx) = tokio::sync::mpsc::channel(1);
        let pump = async {
            super::sse_common::pump_sse(&mut response, &local_tx, &mut handler).await;
            drop(local_tx);
        };
        tokio::join!(pump, forward(rx, &output, &receipt));
        Ok(())
    })
    .await;
    // The transport and permit are gone before forwarding setup/HTTP failures.
    // Natural failures retain their terminal event even under backpressure;
    // cancellation/deadline remains nonblocking after local destruction.
    if let Err(error) = result {
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
