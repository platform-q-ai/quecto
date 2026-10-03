//! One agent's own LLM requests (#2436).
//!
//! `admission.counters` count admission attempts — every one this process
//! made across its quota groups, retries and refusals included — and only
//! for a process bound to an admission authority, so they are no record of
//! an agent's LLM requests or what they spent. Each provider request — an
//! attempt: a retry is a request of its own — is therefore reported once
//! when it ends, by the agent that sent it: what it spent as its provider reported it, how long
//! it took, how it ended, and its number in the agent's own sequence. The
//! agent keeps running totals of the same requests ([`RequestTally`]).
//!
//! Measurements only: never prompt or output content.
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::domain::message::UsageInfo;
use crate::domain::request_observation::RequestTrace;

/// How one provider request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequestOutcome {
    /// It returned a reply.
    Ok,
    /// It failed: a provider or transport error. A retry is the next request.
    Error,
    /// It was dropped while it ran: an abort, a steer, a run deadline, a
    /// shutdown.
    Cancelled,
}

impl RequestOutcome {
    /// The outcome as the wire names it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
        }
    }
}

/// What one request spent, as its provider reported it. The input buckets
/// do not overlap: every adapter normalizes `input_tokens` to the input
/// billed at the full price, so a request's whole input is `input_tokens`
/// plus the cache buckets its provider reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestSpend {
    /// Input tokens at the full price: neither cache reads nor cache writes.
    pub input_tokens: u64,
    /// Input tokens served from the provider's prompt cache; `None` when the
    /// provider did not say.
    pub cached_tokens: Option<u64>,
    /// Input tokens written to the provider's prompt cache (Anthropic's
    /// cache creation); `None` when the provider did not say.
    pub cache_write_tokens: Option<u64>,
    /// Output tokens.
    pub output_tokens: u64,
}

/// The sum of two optional counts: known when either is.
fn either(before: Option<u64>, now: Option<u64>) -> Option<u64> {
    match (before, now) {
        (Some(before), Some(now)) => Some(before.saturating_add(now)),
        (Some(only), None) | (None, Some(only)) => Some(only),
        (None, None) => None,
    }
}

impl RequestSpend {
    /// The sum of `reports`, each counted once; `None` when there are none:
    /// a provider that reported nothing spent nothing we know of.
    pub fn of<'a>(reports: impl IntoIterator<Item = &'a UsageInfo>) -> Option<Self> {
        reports
            .into_iter()
            .fold(None, |spent: Option<Self>, usage| {
                let report = Self {
                    input_tokens: u64::from(usage.prompt_tokens),
                    cached_tokens: usage.cache_read_tokens.map(u64::from),
                    cache_write_tokens: usage.cache_write_tokens.map(u64::from),
                    output_tokens: u64::from(usage.completion_tokens),
                };
                Some(match spent {
                    None => report,
                    Some(spent) => Self {
                        input_tokens: spent.input_tokens.saturating_add(report.input_tokens),
                        cached_tokens: either(spent.cached_tokens, report.cached_tokens),
                        cache_write_tokens: either(
                            spent.cache_write_tokens,
                            report.cache_write_tokens,
                        ),
                        output_tokens: spent.output_tokens.saturating_add(report.output_tokens),
                    },
                })
            })
    }
}

/// One provider request that ended, as its logical request's trace saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndedAttempt {
    /// Its number within its logical request, from 1: 2 and above are
    /// retries.
    pub attempt: u32,
    pub outcome: RequestOutcome,
    /// From when it was admitted (when it started, if it never waited for
    /// admission) to when it ended.
    pub duration_ms: u64,
    /// How long it waited to be admitted; `None` when it never waited (no
    /// admission authority gates its provider).
    pub queued_ms: Option<u64>,
    /// `None` when its provider reported no usage for it.
    pub spend: Option<RequestSpend>,
}

/// One ended provider request, numbered in its agent's own sequence: the
/// `request_completed` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestCompleted {
    /// The model id the provider was sent.
    pub model: String,
    /// The provider the request went to.
    pub provider: String,
    /// `None` when the provider reported no usage for it.
    pub spend: Option<RequestSpend>,
    pub duration_ms: u64,
    pub queued_ms: Option<u64>,
    pub outcome: RequestOutcome,
    /// This agent's requests so far, this one included: 1, 2, 3, …
    pub request_index: u64,
    /// Its number among the attempts of its logical request that were sent,
    /// from 1: the first sent attempt of every request is 1.
    pub attempt: u32,
}

/// This agent's own LLM requests so far and what they spent, as `get_state`
/// reports them (`agentRequests`): not `admission.counters`, which count
/// admission attempts (refusals included) and only when the process is
/// bound to an admission authority. A token count sums what the providers
/// reported; a request that reported nothing adds nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentRequestCounters {
    /// Provider requests ended, retries included: the last `requestIndex`.
    pub requests: u64,
    pub input_tokens: u64,
    pub cached_tokens: u64,
    pub cache_write_tokens: u64,
    pub output_tokens: u64,
}

/// One agent's running count of its requests: the agent loop keeps one for
/// its whole life, and `get_state` reads it.
#[derive(Debug, Default)]
pub struct RequestTally(Mutex<AgentRequestCounters>);

impl RequestTally {
    /// Count `ended`, a request to `provider` for `model`: its record,
    /// numbered next in this agent's sequence.
    pub fn record(&self, model: &str, provider: &str, ended: EndedAttempt) -> RequestCompleted {
        let mut counters = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let before = counters.requests;
        counters.requests = before.saturating_add(1);
        if let Some(spend) = ended.spend {
            counters.input_tokens = counters.input_tokens.saturating_add(spend.input_tokens);
            counters.cached_tokens = counters
                .cached_tokens
                .saturating_add(spend.cached_tokens.unwrap_or(0));
            counters.cache_write_tokens = counters
                .cache_write_tokens
                .saturating_add(spend.cache_write_tokens.unwrap_or(0));
            counters.output_tokens = counters.output_tokens.saturating_add(spend.output_tokens);
        }
        debug_assert!(
            counters.requests > before,
            "every ended request takes the next index"
        );
        RequestCompleted {
            model: model.into(),
            provider: provider.into(),
            spend: ended.spend,
            duration_ms: ended.duration_ms,
            queued_ms: ended.queued_ms,
            outcome: ended.outcome,
            request_index: counters.requests,
            attempt: ended.attempt,
        }
    }

    /// The counts so far.
    pub fn counters(&self) -> AgentRequestCounters {
        *self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Receives each attempt of a request as it ends. Called synchronously, on
/// whichever task ends the attempt: it must not block.
pub type AttemptEndSink = Arc<dyn Fn(EndedAttempt) + Send + Sync>;

/// Where a request's attempts are reported: the first sink attached.
#[derive(Default)]
pub(in crate::domain) struct AttemptEndHook(std::sync::OnceLock<AttemptEndSink>);

impl std::fmt::Debug for AttemptEndHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttemptEndHook")
            .field("attached", &self.0.get().is_some())
            .finish()
    }
}

/// A request's attempt in flight: started and not yet ended.
#[derive(Debug, Default)]
pub(in crate::domain) struct AttemptClock {
    /// The highest attempt number the trace started so far.
    started: u32,
    /// Its attempts reported as sent so far; withdrawn ones are not.
    sent: u32,
    open: Option<OpenAttempt>,
}

#[derive(Debug, Clone, Copy)]
struct OpenAttempt {
    /// When it started, or when it was admitted after waiting.
    since: Instant,
    /// When it began waiting for admission, while it waits: an attempt
    /// that ends still waiting was never sent.
    queued: Option<Instant>,
    /// How long it waited, once admitted.
    queued_ms: Option<u64>,
    /// The usage reports of cut-short attempts recorded before it started:
    /// the ones after are its own.
    usage_mark: usize,
}

fn millis(since: Instant) -> u64 {
    since.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

impl RequestTrace {
    /// Report each attempt of this request to `sink` as it ends; the first
    /// sink attached stays.
    pub fn on_attempt_end(&self, sink: AttemptEndSink) {
        let _ = self.attempt_end.0.set(sink);
    }

    /// Attempt `number` started. One still open ended in error: it was
    /// retried without saying it had ended (or, still waiting for
    /// admission, it was never sent). A number already started is the same
    /// attempt.
    pub(in crate::domain) fn open_attempt(&self, number: u32) {
        debug_assert!(number >= 1, "attempts are numbered from 1");
        let ended = {
            let mut clock = self.clock();
            if number <= clock.started {
                return;
            }
            clock.started = number;
            let previous = clock.open.replace(OpenAttempt {
                since: Instant::now(),
                queued: None,
                queued_ms: None,
                usage_mark: self.unfinished_reports(),
            });
            previous.and_then(|open| self.ended(&mut clock, open, RequestOutcome::Error, None))
        };
        if let Some(ended) = ended {
            self.report(ended);
        }
    }

    /// The attempt in flight waits for admission: until it is admitted it
    /// has not been sent, so it ends withdrawn — however it ends.
    /// An attempt admitted once stays sent (a replay re-admits it).
    pub fn queue_attempt(&self) {
        drop(self.clock());
    }

    /// The attempt in flight was admitted: it is sent now, and its duration
    /// runs from here; its wait is `queued_ms`.
    pub fn admit_attempt(&self) {
        drop(self.clock());
    }

    /// The attempt in flight was never sent: admission refused it, or it was
    /// cancelled while it waited to be admitted. It is withdrawn, never
    /// reported: nothing reached a provider (#2436 review).
    pub fn withdraw_attempt(&self) {
        let mut clock = self.clock();
        if clock.open.take().is_some() {
            clock.sent = clock.sent.saturating_add(1);
        }
    }

    /// The attempt in flight failed: it ends now, as an error, before any
    /// back-off or refresh that precedes its retry.
    pub fn end_attempt_failed(&self) {
        self.end_attempt(RequestOutcome::Error, None);
    }

    /// The attempt in flight ended with `outcome`; `reply` is the usage its
    /// reply reported. Nothing is reported when no attempt is in flight (it
    /// has already ended, or none started: admission refused the request),
    /// or when it still waits for admission: it was never sent.
    pub fn end_attempt(&self, outcome: RequestOutcome, reply: Option<&UsageInfo>) {
        let ended = {
            let mut clock = self.clock();
            let open = clock.open.take();
            open.and_then(|open| self.ended(&mut clock, open, outcome, reply))
        };
        if let Some(ended) = ended {
            self.report(ended);
        }
    }

    /// `open`, ended: numbered next among the sent attempts; `None` when it
    /// was still waiting for admission, so never sent.
    fn ended(
        &self,
        clock: &mut AttemptClock,
        open: OpenAttempt,
        outcome: RequestOutcome,
        reply: Option<&UsageInfo>,
    ) -> Option<EndedAttempt> {
        if open.queued.is_some() {
            return None;
        }
        clock.sent = clock.sent.saturating_add(1);
        let reported = self.unfinished_usage();
        let own = reported.get(open.usage_mark..).unwrap_or_default();
        Some(EndedAttempt {
            attempt: clock.sent,
            outcome,
            duration_ms: millis(open.since),
            queued_ms: open.queued_ms,
            spend: RequestSpend::of(own.iter().chain(reply)),
        })
    }

    fn report(&self, ended: EndedAttempt) {
        if let Some(sink) = self.attempt_end.0.get() {
            sink(ended);
        }
    }

    fn clock(&self) -> std::sync::MutexGuard<'_, AttemptClock> {
        self.attempt_clock.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
#[path = "request_completion_tests.rs"]
mod tests;
