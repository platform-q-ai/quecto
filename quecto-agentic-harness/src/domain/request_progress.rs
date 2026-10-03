//! Live progress of a provider request while it runs (#2210).
//!
//! A model turn once ran for 8+ minutes while its parent saw only
//! `thinking`: nothing told a live but runaway reply from a hung one. The
//! request's trace therefore keeps the attempt in flight — when it started,
//! how many provider events and how many bytes of output it has streamed,
//! when its first token and its last event arrived — and:
//!
//! - `get_state` reads it through [`InFlightRequest`] as `modelTurn`;
//! - the transport stops an attempt whose output passes its cap
//!   ([`output_cap_bytes`], [`OutputCapped`]);
//! - a request that ends while an attempt is still in flight (a run
//!   deadline, an abort, a shutdown) records that attempt as
//!   `Termination::Interrupted` from it.
//!
//! Measurements only: never prompt or output content.
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::attempt_diagnostics::{AttemptDiagnostics, Termination};
use super::provider_error::OUTPUT_CAP_EXCEEDED;
use super::request_observation::{MAX_ATTEMPT_RECORDS, RequestTrace};
pub use super::state_snapshot::{AttemptProgressSnapshot, ModelTurnSnapshot};

/// Bytes of streamed output one attempt may send per token of its output
/// limit. A token is about four bytes of English and rarely more than
/// eight, so a reply within its limit stays well under the cap; a reply
/// over it has run past any limit the provider would enforce — a runaway,
/// such as a repetition loop.
pub const OUTPUT_BYTES_PER_TOKEN: u64 = 8;

/// The output limit assumed when the model's is not known.
pub const FALLBACK_OUTPUT_TOKENS: u32 = 32_768;

/// The output cap of one attempt, in bytes: [`OUTPUT_BYTES_PER_TOKEN`] per
/// token of the model's declared output limit, or of
/// [`FALLBACK_OUTPUT_TOKENS`] when none is known — and never below the
/// request's own `max_tokens`, which a request may raise above either
/// (#2124). The cap is a harness-side bound for providers that enforce none
/// (the Codex ChatGPT backend takes no `max_output_tokens`, #1233).
pub fn output_cap_bytes(model_max_output_tokens: Option<u32>, request_max_tokens: u32) -> u64 {
    let limit = model_max_output_tokens
        .filter(|tokens| *tokens > 0)
        .unwrap_or(FALLBACK_OUTPUT_TOKENS)
        .max(request_max_tokens);
    u64::from(limit) * OUTPUT_BYTES_PER_TOKEN
}

/// An attempt stopped because its output passed its cap: the error it ends
/// with. Classified `output_capped`, never retried: a runaway tends to
/// repeat, each time at the cost of the whole cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputCapped {
    /// The cap, in bytes.
    pub cap: u64,
}

impl std::fmt::Display for OutputCapped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{OUTPUT_CAP_EXCEEDED}the reply streamed more than {} bytes of output in one \
             attempt ({OUTPUT_BYTES_PER_TOKEN} bytes per token of its output limit); the \
             attempt was abandoned as a likely runaway, such as a repetition loop",
            self.cap
        )
    }
}

/// The attempt a request's trace is following: open from its start until
/// its record arrives.
#[derive(Debug, Default)]
pub(super) struct LiveAttempt {
    open: bool,
    number: u32,
    started: Option<Instant>,
    started_unix_ms: u64,
    events: u32,
    output_bytes: u64,
    first_token: Option<Instant>,
    last_event: Option<Instant>,
    /// Its events by type (#2433), for the record of an attempt cut off.
    event_types: super::attempt_diagnostics::EventTypeCounts,
}

/// Milliseconds from `from` to `to`; none when the clock reads earlier.
fn millis(from: Instant, to: Instant) -> u64 {
    u64::try_from(to.saturating_duration_since(from).as_millis()).unwrap_or(u64::MAX)
}

impl RequestTrace {
    /// Cap every attempt of this request at `bytes` of streamed output.
    pub fn set_output_cap(&self, bytes: u64) {
        assert!(bytes > 0, "an output cap is more than zero");
        self.output_cap.store(bytes, Ordering::Relaxed);
    }

    /// The output cap of this request's attempts, when one was set.
    pub fn output_cap(&self) -> Option<u64> {
        Some(self.output_cap.load(Ordering::Relaxed)).filter(|cap| *cap > 0)
    }

    /// An attempt numbered `number` started at `started`: it is the one in
    /// flight, its counts from zero.
    pub fn begin_attempt(&self, number: u32, started: Instant, started_unix_ms: u64) {
        *self.live() = LiveAttempt {
            open: true,
            number,
            started: Some(started),
            started_unix_ms,
            ..LiveAttempt::default()
        };
    }

    /// A provider event arrived at `at` in the attempt in flight, carrying
    /// `output_bytes` of output; `token` when it was output the model
    /// generated (text, thinking or a tool call). The counts are read only
    /// while an attempt is in flight, and each attempt starts them afresh.
    pub fn observe_event(&self, at: Instant, output_bytes: u64, token: bool) {
        let mut live = self.live();
        live.events = live.events.saturating_add(1);
        live.output_bytes = live.output_bytes.saturating_add(output_bytes);
        live.last_event = Some(at);
        if token && live.first_token.is_none() {
            live.first_token = Some(at);
        }
    }

    /// An event of type `kind` arrived in the attempt in flight (#2433).
    pub fn observe_event_kind(&self, kind: &'static str) {
        self.live().event_types.count(kind);
    }

    /// The attempt in flight at `now`, if one is.
    pub fn attempt_progress(&self, now: Instant) -> Option<AttemptProgressSnapshot> {
        let live = self.live();
        let started = live.started.filter(|_| live.open)?;
        Some(AttemptProgressSnapshot {
            number: live.number,
            elapsed_ms: millis(started, now),
            events: live.events,
            output_bytes: live.output_bytes,
            since_last_event_ms: live.last_event.map(|at| millis(at, now)),
            first_token_ms: live.first_token.map(|at| millis(started, at)),
        })
    }

    /// The request is being dropped where it stands (#2210 review): marked
    /// before anything it owns is dropped, so an attempt its transport
    /// records from here on as `Dropped` — dropped with the request — is
    /// recorded as `Interrupted`. An attempt that ended before keeps its own
    /// termination, `Dropped` included.
    pub fn mark_dropping(&self) {
        self.dropping.store(true, Ordering::SeqCst);
    }

    /// The attempts of a request that ended in flight at `now` (`now_unix_ms`
    /// on the wall clock, when readable), read under the live lock so an
    /// attempt ending meanwhile is counted once (#2210 review): each ended
    /// attempt's record, and the one still in flight — a stream running in
    /// its own task — recorded from what it had streamed, as `Interrupted`
    /// (a transport dropped with the request recorded itself so, see
    /// [`Self::mark_dropping`]). At most
    /// [`MAX_ATTEMPT_RECORDS`] records, as for any request.
    pub fn attempts_when_dropped(
        &self,
        now: Instant,
        now_unix_ms: Option<u64>,
    ) -> Vec<AttemptDiagnostics> {
        let live = self.live();
        let mut records = self
            .diagnostics
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(in_flight) = live.interrupted(now, now_unix_ms) {
            if records.len() < MAX_ATTEMPT_RECORDS {
                records.push(in_flight);
            }
        }
        debug_assert!(records.len() <= MAX_ATTEMPT_RECORDS);
        records
    }

    pub(super) fn live(&self) -> std::sync::MutexGuard<'_, LiveAttempt> {
        self.live.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl LiveAttempt {
    /// The attempt numbered `number` ended: it is no longer in flight.
    pub(super) fn close(&mut self, number: u32) {
        if self.open && self.number == number {
            self.open = false;
        }
    }

    /// The record of the attempt in flight when its request ended at `now`:
    /// what it had streamed, ended as `Interrupted`; `None` when no attempt
    /// is in flight.
    fn interrupted(&self, now: Instant, now_unix_ms: Option<u64>) -> Option<AttemptDiagnostics> {
        let started = self.started.filter(|_| self.open)?;
        Some(AttemptDiagnostics {
            attempt_number: self.number,
            started_unix_ms: self.started_unix_ms,
            finished_unix_ms: now_unix_ms.unwrap_or(self.started_unix_ms),
            elapsed_ms: millis(started, now),
            event_count: self.events,
            output_bytes: self.output_bytes,
            first_token_ms: self.first_token.map(|at| millis(started, at)),
            event_types: self.event_types.top(),
            termination: Termination::Interrupted,
            ..AttemptDiagnostics::default()
        })
    }
}

/// The provider request an agent loop has in flight, if any: what
/// `get_state` reports as `modelTurn` while the model thinks or streams.
#[derive(Debug, Default)]
pub struct InFlightRequest {
    current: Mutex<Option<(Instant, Arc<RequestTrace>)>>,
}

impl InFlightRequest {
    /// A request traced by `trace` started at `started`.
    pub fn begin(&self, started: Instant, trace: Arc<RequestTrace>) {
        let mut current = self.current();
        debug_assert!(current.is_none(), "one request in flight at a time");
        *current = Some((started, trace));
    }

    /// The request traced by `trace` ended; another request in flight
    /// stays.
    pub fn end(&self, trace: &Arc<RequestTrace>) {
        let mut current = self.current();
        if current
            .as_ref()
            .is_some_and(|(_, running)| Arc::ptr_eq(running, trace))
        {
            *current = None;
        }
    }

    /// The request in flight at `now`, if one is.
    pub fn snapshot(&self, now: Instant) -> Option<ModelTurnSnapshot> {
        let (started, trace) = self.current().clone()?;
        Some(ModelTurnSnapshot {
            elapsed_ms: millis(started, now),
            output_cap_bytes: trace.output_cap(),
            attempt: trace.attempt_progress(now),
        })
    }

    fn current(&self) -> std::sync::MutexGuard<'_, Option<(Instant, Arc<RequestTrace>)>> {
        self.current.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
#[path = "request_progress_tests.rs"]
mod tests;
