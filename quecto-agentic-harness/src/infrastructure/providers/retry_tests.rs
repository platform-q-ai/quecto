//! Behaviour tests for [`RetryingProvider`] (#931).
//!
//! These assert *behaviour, not mechanics*: a retryable error is retried the
//! expected number of times then succeeds / gives up (attempt count asserted
//! via a counting mock provider, mirroring
//! `refreshable_tests::MockRetryProvider`); a 4xx / cancelled error is NOT
//! retried.

use super::*;
use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::domain::provider::{ChatRequest, LlmProvider};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

/// A mock provider that returns a configurable error message on the first
/// `fail_until` calls, then succeeds. Shares an atomic call counter so tests
/// can assert the exact number of attempts the decorator made.
///
/// Mirrors `refreshable_tests::MockRetryProvider`, but the error text is
/// configurable so we can drive every `ProviderErrorClass`.
#[derive(Debug)]
struct CountingMockProvider {
    call_count: Arc<AtomicU32>,
    fail_until: u32,
    error_message: String,
}

impl CountingMockProvider {
    fn new(call_count: Arc<AtomicU32>, fail_until: u32, error_message: &str) -> Self {
        Self {
            call_count,
            fail_until,
            error_message: error_message.to_string(),
        }
    }
}

impl LlmProvider for CountingMockProvider {
    fn name(&self) -> &str {
        "counting-mock"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        let n = self.call_count.fetch_add(1, Ordering::SeqCst);
        let fail = n < self.fail_until;
        let msg = self.error_message.clone();
        Box::pin(async move {
            if fail {
                Err(DomainError::Provider(msg))
            } else {
                Ok(LlmResponse {
                    content: Some("success".to_string()),
                    tool_calls: vec![],
                    usage: None,
                    stop_reason: None,
                    thinking_blocks: vec![],
                })
            }
        })
    }
}

fn test_request() -> ChatRequest<'static> {
    ChatRequest {
        messages: &[],
        tools: &[],
        model: "test-model",
        max_tokens: 1024,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
}

// ── Acceptance criterion 1: retryable errors are retried with bounded attempts ─

#[tokio::test]
async fn test_retries_server_error_then_succeeds() {
    let count = Arc::new(AtomicU32::new(0));
    // Fail twice with a 503, then succeed on the third attempt.
    let inner = Arc::new(CountingMockProvider::new(
        count.clone(),
        2,
        "provider error (503): service unavailable",
    ));
    let retrying = RetryingProvider::new(inner, RetryConfig::no_delay(4));

    let result = retrying.chat(test_request()).await;
    assert!(result.is_ok(), "should succeed after retries: {:?}", result);
    assert_eq!(
        count.load(Ordering::SeqCst),
        3,
        "expected 3 attempts (2 failures + 1 success)"
    );
}

#[tokio::test]
async fn test_retries_rate_limit_then_succeeds() {
    let count = Arc::new(AtomicU32::new(0));
    let inner = Arc::new(CountingMockProvider::new(
        count.clone(),
        1,
        "provider error (429): rate limit exceeded",
    ));
    let retrying = RetryingProvider::new(inner, RetryConfig::no_delay(4));

    let result = retrying.chat(test_request()).await;
    assert!(result.is_ok(), "429 should be retried: {:?}", result);
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_retries_network_error_then_succeeds() {
    let count = Arc::new(AtomicU32::new(0));
    let inner = Arc::new(CountingMockProvider::new(
        count.clone(),
        2,
        "connection reset by peer",
    ));
    let retrying = RetryingProvider::new(inner, RetryConfig::no_delay(4));

    let result = retrying.chat(test_request()).await;
    assert!(
        result.is_ok(),
        "network error should be retried: {:?}",
        result
    );
    assert_eq!(count.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn test_gives_up_after_max_attempts() {
    let count = Arc::new(AtomicU32::new(0));
    // Always fails with a retryable 500.
    let inner = Arc::new(CountingMockProvider::new(
        count.clone(),
        u32::MAX,
        "provider error (500): internal server error",
    ));
    let retrying = RetryingProvider::new(inner, RetryConfig::no_delay(3));

    let result = retrying.chat(test_request()).await;
    assert!(result.is_err(), "should give up and return the error");
    assert_eq!(
        count.load(Ordering::SeqCst),
        3,
        "should attempt exactly max_attempts (3) times before giving up"
    );
}

// ── Acceptance criterion 1: non-retryable errors are NOT retried ───────────────

#[tokio::test]
async fn test_does_not_retry_client_4xx() {
    let count = Arc::new(AtomicU32::new(0));
    let inner = Arc::new(CountingMockProvider::new(
        count.clone(),
        u32::MAX,
        "provider error (400): invalid_request_error",
    ));
    let retrying = RetryingProvider::new(inner, RetryConfig::no_delay(4));

    let result = retrying.chat(test_request()).await;
    assert!(result.is_err(), "4xx must surface as an error");
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "Client 4xx must NOT be retried — exactly one attempt"
    );
}

#[tokio::test]
async fn test_does_not_retry_auth_401() {
    let count = Arc::new(AtomicU32::new(0));
    let inner = Arc::new(CountingMockProvider::new(
        count.clone(),
        u32::MAX,
        "provider error (401): unauthorized",
    ));
    let retrying = RetryingProvider::new(inner, RetryConfig::no_delay(4));

    let result = retrying.chat(test_request()).await;
    assert!(result.is_err());
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "Auth 401 must NOT be retried by the retry decorator"
    );
}

#[tokio::test]
async fn test_does_not_retry_cancelled() {
    let count = Arc::new(AtomicU32::new(0));
    let inner = Arc::new(CountingMockProvider::new(
        count.clone(),
        u32::MAX,
        "request cancelled",
    ));
    let retrying = RetryingProvider::new(inner, RetryConfig::no_delay(4));

    let result = retrying.chat(test_request()).await;
    assert!(result.is_err());
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "Cancelled must NOT be retried"
    );
}

#[tokio::test]
async fn test_success_is_single_attempt() {
    let count = Arc::new(AtomicU32::new(0));
    let inner = Arc::new(CountingMockProvider::new(count.clone(), 0, "unused"));
    let retrying = RetryingProvider::new(inner, RetryConfig::no_delay(4));

    let result = retrying.chat(test_request()).await;
    assert!(result.is_ok());
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "a successful call must not retry"
    );
}

// ── Acceptance criterion 1: a Retry-After hint is honoured for the delay ───────

/// A sleep seam that records every requested delay instead of sleeping, so the
/// chosen backoff is observable (the only way to assert `Retry-After` honouring).
fn recording_sleeper() -> (SleepFn, Arc<std::sync::Mutex<Vec<Duration>>>) {
    let log: Arc<std::sync::Mutex<Vec<Duration>>> = Arc::new(std::sync::Mutex::new(vec![]));
    let log_for_fn = log.clone();
    let sleeper: SleepFn = Arc::new(move |d: Duration| {
        log_for_fn.lock().unwrap().push(d);
        Box::pin(async {}) as Pin<Box<dyn Future<Output = ()> + Send>>
    });
    (sleeper, log)
}

#[tokio::test]
async fn test_honours_retry_after_ms_hint_over_exponential_default() {
    let count = Arc::new(AtomicU32::new(0));
    // 429 with an explicit retry-after-ms hint, then succeed.
    let inner = Arc::new(CountingMockProvider::new(
        count.clone(),
        1,
        "provider error (429): rate limit exceeded, retry-after-ms: 1234",
    ));
    // Base backoff is large so the exponential default (>=2s) is clearly
    // distinguishable from the 1234ms hint.
    let config = RetryConfig {
        max_attempts: 4,
        base_backoff: Duration::from_secs(2),
        max_backoff: Duration::from_secs(30),
    };
    let (sleeper, delays) = recording_sleeper();
    let retrying = RetryingProvider::with_sleeper(inner, config, sleeper);

    let result = retrying.chat(test_request()).await;
    assert!(result.is_ok(), "429 should be retried: {:?}", result);

    let recorded = delays.lock().unwrap().clone();
    assert_eq!(recorded.len(), 1, "exactly one backoff before the retry");
    assert_eq!(
        recorded[0],
        Duration::from_millis(1234),
        "the Retry-After hint must drive the delay, not the exponential default"
    );
}

#[tokio::test]
async fn test_honours_retry_after_seconds_hint() {
    let count = Arc::new(AtomicU32::new(0));
    let inner = Arc::new(CountingMockProvider::new(
        count.clone(),
        1,
        "provider error (529): overloaded_error, retry-after: 7",
    ));
    // A real `max_backoff` (well above the 7s hint) so the hint is honoured
    // verbatim; `base_backoff` is zero so the exponential default is 0 and the
    // 7s value can only come from the hint path.
    let config = RetryConfig {
        max_attempts: 4,
        base_backoff: Duration::ZERO,
        max_backoff: Duration::from_secs(30),
    };
    let (sleeper, delays) = recording_sleeper();
    let retrying = RetryingProvider::with_sleeper(inner, config, sleeper);

    let result = retrying.chat(test_request()).await;
    assert!(result.is_ok());
    assert_eq!(
        delays.lock().unwrap().clone(),
        vec![Duration::from_secs(7)],
        "a `Retry-After` seconds hint must be honoured"
    );
}

#[tokio::test]
async fn test_oversized_retry_after_hint_is_clamped_to_max_backoff() {
    let count = Arc::new(AtomicU32::new(0));
    // A hostile/buggy provider returns an enormous Retry-After. It must be
    // clamped to max_backoff, never block for the untrusted duration.
    let inner = Arc::new(CountingMockProvider::new(
        count.clone(),
        1,
        "provider error (429): rate limit exceeded, retry-after: 999999999",
    ));
    let config = RetryConfig {
        max_attempts: 4,
        base_backoff: Duration::from_millis(500),
        max_backoff: Duration::from_secs(30),
    };
    let (sleeper, delays) = recording_sleeper();
    let retrying = RetryingProvider::with_sleeper(inner, config, sleeper);

    let result = retrying.chat(test_request()).await;
    assert!(result.is_ok());
    assert_eq!(
        delays.lock().unwrap().clone(),
        vec![Duration::from_secs(30)],
        "an oversized Retry-After hint must be clamped to max_backoff"
    );
}

// ── Design constraint: streaming initiation is not retried by the decorator ────

#[tokio::test]
async fn test_stream_initiation_failure_is_not_retried() {
    let count = Arc::new(AtomicU32::new(0));
    // Always fails with a retryable 503. If the decorator retried streaming it
    // would call the inner provider more than once.
    let inner = Arc::new(CountingMockProvider::new(
        count.clone(),
        u32::MAX,
        "provider error (503): service unavailable",
    ));
    let retrying = RetryingProvider::new(inner, RetryConfig::no_delay(4));

    let result = retrying.chat_stream(test_request()).await;
    assert!(result.is_err());
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "streaming must not be retried mid-flight — exactly one initiation attempt"
    );
}

#[tokio::test]
async fn test_incremental_stream_initiation_failure_is_not_retried() {
    let count = Arc::new(AtomicU32::new(0));
    let inner = Arc::new(CountingMockProvider::new(
        count.clone(),
        u32::MAX,
        "provider error (503): service unavailable",
    ));
    let retrying = RetryingProvider::new(inner, RetryConfig::no_delay(4));

    let mut rx = retrying.chat_stream_incremental(test_request()).await;
    match rx.recv().await.expect("default incremental error event") {
        crate::domain::provider::StreamEvent::Error(err) => {
            assert!(err.contains("service unavailable"), "{err}");
        }
        other => panic!("unexpected stream event: {other:?}"),
    }
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "incremental streaming must not be retried by the retry decorator"
    );
}

#[tokio::test]
async fn counting_mock_provider_trait_surface_defaults_are_exercised() {
    let count = Arc::new(AtomicU32::new(0));
    let provider = CountingMockProvider::new(count, 0, "provider error (500): unused");

    assert_eq!(provider.name(), "counting-mock");
    assert!(provider.as_any().downcast_ref::<()>().is_some());

    let response = provider.chat_stream(test_request()).await.unwrap();
    assert_eq!(response.content.as_deref(), Some("success"));

    let mut rx = provider.chat_stream_incremental(test_request()).await;
    match rx.recv().await.expect("default stream event") {
        crate::domain::provider::StreamEvent::Done(done) => {
            assert_eq!(done.content.as_deref(), Some("success"));
        }
        other => panic!("unexpected stream event: {other:?}"),
    }
    assert!(rx.recv().await.is_none());
}

#[test]
fn wave3_debug_and_jitter_zero_path() {
    let count = Arc::new(AtomicU32::new(0));
    let inner = Arc::new(CountingMockProvider::new(count, 0, "unused"));
    let retrying = RetryingProvider::new(inner, RetryConfig::no_delay(2));
    let dbg = format!("{retrying:?}");
    assert!(dbg.contains("RetryingProvider"));
    assert!(dbg.contains("max_attempts"));
    assert_eq!(jitter(std::time::Duration::ZERO), std::time::Duration::ZERO);
}
