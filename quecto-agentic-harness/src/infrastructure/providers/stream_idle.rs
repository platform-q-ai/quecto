//! A bound on a silent provider stream (#2210).
//!
//! A streaming reply may legitimately run for many minutes, so no provider
//! request has a total time limit. What must never happen is a reply that
//! sends *nothing* for ever: a hung connection or a stalled server held a
//! turn for 8+ minutes with the parent seeing only `thinking`. Every step
//! of a streaming exchange — the send, until the response head arrives, and
//! each read of the body after it — is therefore bounded by
//! [`STREAM_IDLE_LIMIT`]. The send is bounded from the send; an SSE body
//! from its last *event* ([`EventIdle`], #2433), so keep-alives alone never
//! hold a reply open; any other body from its last bytes. A long reply that
//! keeps sending events is never cut short.
//!
//! A whole non-streaming reply sends nothing until complete, so it has a
//! total bound instead, [`REPLY_TOTAL_LIMIT`].
//!
//! An expiry is [`Idle`] (a stream idle timeout, `Termination::Idle`) or
//! [`TimedOut`] (a reply timeout, `Termination::TimedOut`); the retry
//! classifier treats either as `Stalled`, retried at most once. Each
//! provider carries its [`StreamIdle`] bounds, so tests can shorten them
//! without a global.
use std::future::Future;
use std::time::Duration;

use crate::domain::provider_error::{REPLY_TIMEOUT, STREAM_IDLE_TIMEOUT};

/// How long a streaming reply may send nothing — no response head, no SSE
/// event (a keep-alive is no event, #2433) — before its request is abandoned.
///
/// Five minutes: the default stream idle timeout of the official Codex
/// client against the same Responses backend. That backend documents no
/// keep-alive event: while a reasoning model thinks before its first output
/// the stream can be silent (reasoning summaries arrive only as sections
/// finish), and so can OpenAI-compatible chat streams of reasoning models.
/// A high-effort turn can think silently for minutes, and an abandoned turn
/// is retried from the start, so a bound a legitimate think can reach would
/// repeat that think on every attempt and then fail it. Five minutes still
/// ends a stalled stream well before any outer deadline would. Anthropic
/// streams `ping` events, so there it only ever ends a stream that stopped.
pub const STREAM_IDLE_LIMIT: Duration = Duration::from_secs(300);

/// How long a whole non-streaming reply may take, from the send to its last
/// byte, before its request is abandoned (#2210 review).
///
/// A non-streaming reply sends nothing until it is complete, so it cannot
/// have an idle bound; only a total one. Twenty minutes: a reply of a
/// model's whole output (64k tokens at a slow 60 tokens/s is about 18
/// minutes) still arrives, while a one-shot `quecto agent -m`, which uses
/// this path, can no longer wait for ever.
pub const REPLY_TOTAL_LIMIT: Duration = Duration::from_secs(20 * 60);

/// The bounds one provider applies to its replies: [`STREAM_IDLE_LIMIT`]
/// between the bytes of a streaming reply, and [`REPLY_TOTAL_LIMIT`] over a
/// whole non-streaming one, unless shorter ones were chosen (tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamIdle {
    idle: Duration,
    total: Duration,
}

impl Default for StreamIdle {
    fn default() -> Self {
        Self {
            idle: STREAM_IDLE_LIMIT,
            total: REPLY_TOTAL_LIMIT,
        }
    }
}

impl StreamIdle {
    /// An idle bound of `limit`, which must be more than zero: a zero bound
    /// would abandon every request before its first byte.
    pub fn new(limit: Duration) -> Self {
        assert!(!limit.is_zero(), "a stream idle limit is more than zero");
        Self {
            idle: limit,
            ..Self::default()
        }
    }

    /// The same bounds with a total bound of `total`, more than zero.
    pub fn with_total(self, total: Duration) -> Self {
        assert!(!total.is_zero(), "a reply total limit is more than zero");
        Self { total, ..self }
    }

    /// The idle bound.
    pub fn limit(self) -> Duration {
        self.idle
    }

    /// The total bound.
    pub fn total(self) -> Duration {
        self.total
    }

    /// Await one step of a streaming exchange (the send, or one read of the
    /// body), abandoning it as [`Idle`] once it has waited the whole bound.
    pub async fn within<F: Future>(self, step: F) -> Result<F::Output, Idle> {
        match tokio::time::timeout(self.idle, step).await {
            Ok(output) => Ok(output),
            Err(_elapsed) => Err(Idle(self.idle)),
        }
    }

    /// Await a whole non-streaming exchange, abandoning it as [`TimedOut`]
    /// once it has taken the whole total bound.
    pub async fn whole<F: Future>(self, exchange: F) -> Result<F::Output, TimedOut> {
        match tokio::time::timeout(self.total, exchange).await {
            Ok(output) => Ok(output),
            Err(_elapsed) => Err(TimedOut(self.total)),
        }
    }

    /// Read a whole response body as text, each read bounded from the last.
    /// The bytes are decoded as UTF-8, lossily, whatever charset the
    /// response names: the bodies read here are error bodies, shown as lossy
    /// UTF-8; an SSE body is read by [`Self::sse_text`].
    pub async fn text(self, response: reqwest::Response) -> Result<String, BodyError> {
        read_whole(response, EventIdle::reads(self)).await
    }

    /// Read a whole SSE body as text, bounded from its last event
    /// ([`EventIdle`], #2433): a body that only keeps alive is idle.
    pub async fn sse_text(self, response: reqwest::Response) -> Result<String, BodyError> {
        read_whole(response, EventIdle::events(self)).await
    }
}

/// Read a whole body as lossy UTF-8, each read bounded by `idle`.
async fn read_whole(
    mut response: reqwest::Response,
    mut idle: EventIdle,
) -> Result<String, BodyError> {
    let mut body = Vec::new();
    loop {
        match idle.next(response.chunk()).await {
            Ok(Ok(Some(bytes))) => body.extend_from_slice(&bytes),
            Ok(Ok(None)) => return Ok(String::from_utf8_lossy(&body).into_owned()),
            Ok(Err(error)) => return Err(BodyError::Read(error)),
            Err(idle) => return Err(BodyError::Idle(idle)),
        }
    }
}

/// The bytes that open an SSE event's data line (#2433).
const EVENT_LINE: &[u8] = b"data:";

/// The idle bound of a response body, measured from its last progress
/// (#2433).
///
/// For an SSE body, progress is an event: the bytes of a `data:` line. A
/// keep-alive — an SSE comment, a blank line, an empty chunk — shows the
/// connection is up, not that the reply is moving. A Codex reply showed no
/// progress for 15 minutes, its body still being read, and a bound that any
/// bytes restarted never ended it. Only events restart the bound, as the official Codex
/// client's idle timeout runs between events; a reply that keeps sending
/// events (Anthropic's `ping` events included) is never cut short. For any
/// other body (an error body), every read is progress.
#[derive(Debug)]
pub struct EventIdle {
    bound: Duration,
    deadline: tokio::time::Instant,
    /// `None` when every read is progress; otherwise where the body's
    /// current line stands.
    line: Option<Line>,
}

/// Where an SSE body's current line stands, as far as its start shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    /// Its first `n` bytes open [`EVENT_LINE`].
    Opening(usize),
    /// An event's data line.
    Event,
    /// Any other line: a comment, another field, a blank line.
    Other,
}

impl EventIdle {
    /// The bound of an SSE body starting now: restarted by its events.
    pub fn events(bound: StreamIdle) -> Self {
        Self::starting(bound, Some(Line::Opening(0)))
    }

    /// The bound of any other body starting now: restarted by every read.
    pub fn reads(bound: StreamIdle) -> Self {
        Self::starting(bound, None)
    }

    fn starting(bound: StreamIdle, line: Option<Line>) -> Self {
        Self {
            bound: bound.idle,
            deadline: tokio::time::Instant::now() + bound.idle,
            line,
        }
    }

    /// Await one read of the body, abandoning it as [`Idle`] once the whole
    /// bound has passed since the body's last progress (or its start), and
    /// observe what it read.
    pub async fn next<B: AsRef<[u8]>, E>(
        &mut self,
        read: impl Future<Output = Result<Option<B>, E>>,
    ) -> Result<Result<Option<B>, E>, Idle> {
        let read = match tokio::time::timeout_at(self.deadline, read).await {
            Ok(read) => read,
            Err(_elapsed) => return Err(Idle(self.bound)),
        };
        if let Ok(Some(bytes)) = &read {
            self.observe(bytes.as_ref());
        }
        Ok(read)
    }

    /// Bytes of the body arrived: progress restarts the bound.
    fn observe(&mut self, bytes: &[u8]) {
        let progress = match &mut self.line {
            None => true,
            Some(line) => bytes.iter().fold(false, |progress, &byte| {
                *line = line.after(byte);
                progress || *line == Line::Event
            }),
        };
        if progress {
            self.deadline = tokio::time::Instant::now() + self.bound;
        }
    }
}

impl Line {
    /// Where the line stands after `byte`; a newline starts the next line.
    fn after(self, byte: u8) -> Self {
        match (self, byte) {
            (_, b'\n') => Self::Opening(0),
            (Self::Opening(opened), byte) if byte == EVENT_LINE[opened] => {
                match opened + 1 == EVENT_LINE.len() {
                    true => Self::Event,
                    false => Self::Opening(opened + 1),
                }
            }
            (Self::Opening(_), _) => Self::Other,
            (line @ (Self::Event | Self::Other), _) => line,
        }
    }
}

/// The provider sent no event (no response head, before one) for the bound
/// it carries; the request was abandoned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Idle(Duration);

impl std::fmt::Display for Idle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{STREAM_IDLE_TIMEOUT}the provider sent no event for {}; the request was abandoned",
            Span(self.0)
        )
    }
}

impl Idle {
    /// What stands for an error body the provider stopped sending: the
    /// status before it still decides the error's class.
    pub fn body_marker(self) -> String {
        let name = STREAM_IDLE_TIMEOUT.trim_end_matches(": ");
        format!("(error body abandoned: {name} after {})", Span(self.0))
    }
}

/// A whole non-streaming reply did not arrive within the total bound it
/// carries; the request was abandoned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimedOut(Duration);

impl std::fmt::Display for TimedOut {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{REPLY_TIMEOUT}the provider sent no whole reply within {}; the request was abandoned",
            Span(self.0)
        )
    }
}

/// A bound as a message shows it: whole seconds, or milliseconds.
struct Span(Duration);

impl std::fmt::Display for Span {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0.subsec_nanos() {
            0 => write!(f, "{} s", self.0.as_secs()),
            _ => write!(f, "{} ms", self.0.as_millis()),
        }
    }
}

/// An error body as read: the text, the marker for one the provider stopped
/// sending, or nothing when the transport failed (as before #2210).
pub fn error_text(read: Result<String, BodyError>) -> String {
    match read {
        Ok(text) => text,
        Err(BodyError::Idle(idle)) => idle.body_marker(),
        Err(BodyError::Read(_)) => String::new(),
    }
}

/// Why a whole body could not be read.
#[derive(Debug)]
pub enum BodyError {
    /// The provider went silent mid-body.
    Idle(Idle),
    /// The transport failed.
    Read(reqwest::Error),
}

impl std::fmt::Display for BodyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Idle(idle) => idle.fmt(f),
            Self::Read(error) => error.fmt(f),
        }
    }
}

#[cfg(test)]
#[path = "stream_idle_tests.rs"]
pub(crate) mod tests;
