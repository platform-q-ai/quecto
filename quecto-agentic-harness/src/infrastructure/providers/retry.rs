//! RetryingProvider — decorator that retries transient/retryable provider
//! errors with bounded exponential backoff + jitter.
//!
//! Wraps an inner `LlmProvider` and, driven by `classify_provider_error`,
//! retries `RateLimit` (429), `Server` (5xx/529/overloaded) and `Network`
//! failures up to a bounded number of attempts before giving up. `Client`
//! (4xx), `Auth` and `Cancelled` errors are passed straight through and never
//! retried. When a `Retry-After` / `retry-after-ms` hint is present on a 429/529
//! it is honoured for the backoff delay instead of the exponential default
//! (clamped to `max_backoff`, since the hint comes from an untrusted string).
//!
//! Only the non-streaming `chat()` path is retried: a stream cannot be replayed
//! mid-flight, so `chat_stream` / `chat_stream_incremental` retry nothing here
//! (the stream *initiation* is retried by the agent loop, which owns the
//! emitted-event bookkeeping).
//!
//! Sibling to `refreshable.rs`; composed with the `refreshable` and `router`
//! decorators (issue #931).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use crate::domain::catalogue::ModelDescriptor;
use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::domain::provider::{ChatRequest, LlmProvider};
use crate::domain::provider_error::classify_provider_error;

/// Async sleep seam. Defaults to `tokio::time::sleep`; tests inject a recorder
/// that captures the requested delay (so `Retry-After` honouring is observable)
/// and returns immediately.
pub type SleepFn = Arc<dyn Fn(Duration) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

fn default_sleeper() -> SleepFn {
    Arc::new(|d: Duration| {
        Box::pin(tokio::time::sleep(d)) as Pin<Box<dyn Future<Output = ()> + Send>>
    })
}

/// Configuration for [`RetryingProvider`] backoff behaviour.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum total attempts (initial call + retries).
    pub max_attempts: u32,
    /// Base backoff delay; doubled each retry (exponential), plus jitter.
    pub base_backoff: Duration,
    /// Upper bound for a single backoff delay.
    pub max_backoff: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            base_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(30),
        }
    }
}

impl RetryConfig {
    /// A fast config for tests: no real sleeping between attempts.
    pub fn no_delay(max_attempts: u32) -> Self {
        Self {
            max_attempts,
            base_backoff: Duration::ZERO,
            max_backoff: Duration::ZERO,
        }
    }
}

/// Provider decorator that retries retryable errors with bounded backoff.
pub struct RetryingProvider {
    inner: Arc<dyn LlmProvider>,
    config: RetryConfig,
    sleeper: SleepFn,
}

impl std::fmt::Debug for RetryingProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RetryingProvider")
            .field("config", &self.config)
            .finish()
    }
}

impl RetryingProvider {
    /// Create a new retrying provider wrapping `inner`.
    pub fn new(inner: Arc<dyn LlmProvider>, config: RetryConfig) -> Self {
        Self {
            inner,
            config,
            sleeper: default_sleeper(),
        }
    }

    /// The wrapped inner provider. Lets callers introspect through the
    /// decorator (e.g. downcast to the underlying router).
    pub fn inner(&self) -> &Arc<dyn LlmProvider> {
        &self.inner
    }

    /// Create a retrying provider with a custom sleep seam (used by tests to
    /// observe the chosen backoff delay without sleeping for real).
    pub fn with_sleeper(
        inner: Arc<dyn LlmProvider>,
        config: RetryConfig,
        sleeper: SleepFn,
    ) -> Self {
        Self {
            inner,
            config,
            sleeper,
        }
    }

    /// The delay to wait before the next attempt. Honours a `Retry-After` /
    /// `retry-after-ms` hint on the error when present; otherwise uses bounded
    /// exponential backoff (`base * 2^(attempt-1)`, capped) plus jitter.
    fn backoff_delay(&self, attempt: u32, err: &DomainError) -> Duration {
        if let DomainError::Provider(msg) = err {
            if let Some(hint) = parse_retry_after(&msg.to_ascii_lowercase()) {
                // Clamp the provider-supplied hint to `max_backoff`. The value is
                // parsed from an untrusted provider error string, so a hostile or
                // buggy endpoint emitting e.g. `retry-after: 999999999` must not
                // be able to block the turn in `sleep` past the bounded ceiling.
                return hint.min(self.config.max_backoff);
            }
        }
        let exp = self
            .config
            .base_backoff
            .saturating_mul(2u32.saturating_pow(attempt.saturating_sub(1)));
        let capped = exp.min(self.config.max_backoff);
        // Apply the ceiling *after* jitter so a single delay never exceeds
        // `max_backoff` (the documented bound).
        capped
            .saturating_add(jitter(self.config.base_backoff))
            .min(self.config.max_backoff)
    }
}

/// Pseudo-random jitter in `[0, base)` derived from the wall clock. Avoids a
/// `rand` dependency; jitter only spreads retries and need not be cryptographic.
fn jitter(base: Duration) -> Duration {
    let base_ms = base.as_millis() as u64;
    if base_ms == 0 {
        return Duration::ZERO;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as u64)
        .unwrap_or(0);
    Duration::from_millis(nanos % base_ms)
}

/// Parse a `retry-after-ms` (milliseconds) or `retry-after` (seconds) hint from
/// an already-lowercased provider error string.
fn parse_retry_after(lowered: &str) -> Option<Duration> {
    if let Some(ms) = number_after(lowered, "retry-after-ms") {
        return Some(Duration::from_millis(ms));
    }
    if let Some(secs) = number_after(lowered, "retry-after") {
        return Some(Duration::from_secs(secs));
    }
    None
}

/// Find `marker` in `s`, then parse the run of digits that follows (skipping
/// separators like `:`, `=`, whitespace and quotes).
fn number_after(s: &str, marker: &str) -> Option<u64> {
    let rel = s.find(marker)?;
    let rest = &s[rel + marker.len()..];
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_digit() {
            let mut value: u64 = 0;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                value = value
                    .saturating_mul(10)
                    .saturating_add((bytes[i] - b'0') as u64);
                i += 1;
            }
            return Some(value);
        }
        if matches!(b, b':' | b'=' | b'"' | b'\'' | b' ' | b'\t') {
            i += 1;
            continue;
        }
        return None;
    }
    None
}

impl LlmProvider for RetryingProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn model_descriptors(&self) -> Option<&[ModelDescriptor]> {
        self.inner.model_descriptors()
    }

    fn chat<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        Box::pin(async move {
            let mut attempt: u32 = 1;
            loop {
                // Shallow clone (slice pointers + small Option fields) — the
                // borrow lives for 'a so each attempt can re-forward it.
                match self.inner.chat(request.clone()).await {
                    Ok(response) => return Ok(response),
                    Err(err) => {
                        let class = classify_provider_error(&err);
                        if !class.is_retryable() || attempt >= self.config.max_attempts {
                            return Err(err);
                        }
                        let delay = self.backoff_delay(attempt, &err);
                        tracing::warn!(
                            target: "provider_retry",
                            attempt,
                            max_attempts = self.config.max_attempts,
                            error_class = %class,
                            delay_ms = delay.as_millis() as u64,
                            "retrying provider request after transient failure"
                        );
                        (self.sleeper)(delay).await;
                        attempt += 1;
                    }
                }
            }
        })
    }

    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        // A stream cannot be replayed mid-flight; the agent loop owns initiation
        // retry. Forward straight through.
        Box::pin(async move { self.inner.chat_stream(request).await })
    }

    fn chat_stream_incremental<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<
        Box<
            dyn Future<Output = tokio::sync::mpsc::Receiver<crate::domain::provider::StreamEvent>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move { self.inner.chat_stream_incremental(request).await })
    }
}

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;
