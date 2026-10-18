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
//! hold a reply open; any other body from its last bytes. An SSE body that
//! keeps sending events none of which carries output is bounded by
//! [`STREAM_PROGRESS_LIMIT`] from its last output (#2433). A long reply
//! that keeps sending output is never cut short.
//!
//! A whole non-streaming reply sends nothing until complete, so it has a
//! total bound instead, [`REPLY_TOTAL_LIMIT`].
//!
//! An expiry is [`Idle`] (a stream idle timeout, `Termination::Idle`; or a
//! stream progress timeout, `Termination::NoProgress`) or [`TimedOut`] (a
//! reply timeout, `Termination::TimedOut`); the retry classifier treats
//! each as `Stalled`, retried at most once. Each
//! provider carries its [`StreamIdle`] bounds, so tests can shorten them
//! without a global.
use std::future::Future;
use std::time::Duration;

use crate::domain::inference::services::provider_error::{
    REPLY_TIMEOUT, STREAM_IDLE_TIMEOUT, STREAM_PROGRESS_TIMEOUT,
};

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

/// The stream idle limits, in seconds, a provider may be configured with
/// (`stream_idle_seconds`, #2433 review): from half a minute — any shorter
/// would cut an ordinary reasoning pause — to half an hour, beyond which a
/// stall is held longer than the admission attempt timeout's default.
pub const STREAM_IDLE_SECONDS: std::ops::RangeInclusive<u64> = 30..=1800;

/// How long a streaming reply may keep sending events none of which carries
/// output — no text, reasoning, tool call or completion
/// ([`super::sse_progress`]) — before its request is abandoned (#2433).
///
/// A Codex reply streamed ~11 recognised events a second for 15 minutes
/// with no output: every event restarted the idle bound, so nothing ended
/// it. Five minutes, as the idle bound: a model that reasons streams its
/// reasoning, which is output, and one that thinks in hiding sends too few
/// events ([`PROGRESS_EVENTS`]), so only a reply that sends many events but
/// makes nothing of them reaches it.
pub const STREAM_PROGRESS_LIMIT: Duration = Duration::from_secs(300);

/// How many events without output must come since the last output (or the
/// body's start) before the progress bound may end a reply (#2433 review).
///
/// A stall makes progress-free events fast: the reported one ran at ~11 a
/// second, ~3,300 in 300 s. A reply that is thinking makes few: Anthropic
/// streams a `ping` about every 15 s (~20 in 300 s) while it thinks in
/// hiding, and a Codex reply opens with about 3 before a silent think. Two
/// hundred sits far from both, so the count rule never cuts a think at the
/// progress limit. A silent think is the idle bound's alone
/// (`stream_idle_seconds`). A ping-only think is not: its pings are events,
/// restarting the idle bound, and would reach 200 only after ~50 minutes;
/// the backstop ([`PROGRESS_BACKSTOP_FACTOR`]) ends it at 15 minutes by
/// default, and `stream_progress_seconds` is the setting that lets it run.
pub const PROGRESS_EVENTS: u32 = 200;

/// The stream progress limits, in seconds, a provider may be configured
/// with (`stream_progress_seconds`, #2433): from a minute to an hour.
pub const STREAM_PROGRESS_SECONDS: std::ops::RangeInclusive<u64> = 60..=3600;

/// One provider's configured stream limits, in seconds: unset is the
/// default (#2433 review).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StreamLimits {
    /// `stream_idle_seconds`, within [`STREAM_IDLE_SECONDS`].
    pub idle_seconds: Option<u64>,
    /// `stream_progress_seconds`, within [`STREAM_PROGRESS_SECONDS`].
    pub progress_seconds: Option<u64>,
}

/// A configured stream limit outside its allowed range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamLimitError {
    /// `stream_idle_seconds` was this.
    Idle(u64),
    /// `stream_progress_seconds` was this.
    Progress(u64),
}

impl StreamLimitError {
    /// The setting out of range, in `models.json`'s spelling.
    pub fn json_name(self) -> &'static str {
        match self {
            Self::Idle(_) => "streamIdleSeconds",
            Self::Progress(_) => "streamProgressSeconds",
        }
    }
}

impl std::fmt::Display for StreamLimitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (name, range, got) = match *self {
            Self::Idle(got) => ("stream_idle_seconds", STREAM_IDLE_SECONDS, got),
            Self::Progress(got) => ("stream_progress_seconds", STREAM_PROGRESS_SECONDS, got),
        };
        write!(
            f,
            "{name} must be within {}–{} seconds, got {got}",
            range.start(),
            range.end()
        )
    }
}

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
/// between the events of a streaming reply, [`STREAM_PROGRESS_LIMIT`]
/// between its events that carry output, and [`REPLY_TOTAL_LIMIT`] over a
/// whole non-streaming one, unless others were configured or chosen (tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamIdle {
    idle: Duration,
    total: Duration,
    progress: Duration,
}

impl Default for StreamIdle {
    fn default() -> Self {
        Self {
            idle: STREAM_IDLE_LIMIT,
            total: REPLY_TOTAL_LIMIT,
            progress: STREAM_PROGRESS_LIMIT,
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

    /// The bounds of a provider configured with `limits`: each default when
    /// unset, the one it names when within its range, and the error naming
    /// the first out of range otherwise.
    pub fn configured(limits: StreamLimits) -> Result<Self, StreamLimitError> {
        let mut bounds = Self::default();
        if let Some(seconds) = limits.idle_seconds {
            match STREAM_IDLE_SECONDS.contains(&seconds) {
                true => bounds.idle = Duration::from_secs(seconds),
                false => return Err(StreamLimitError::Idle(seconds)),
            }
        }
        if let Some(seconds) = limits.progress_seconds {
            match STREAM_PROGRESS_SECONDS.contains(&seconds) {
                true => bounds.progress = Duration::from_secs(seconds),
                false => return Err(StreamLimitError::Progress(seconds)),
            }
        }
        Ok(bounds)
    }

    /// [`Self::configured`] for the provider setting named `setting`, whose
    /// error names it.
    pub fn configured_for(setting: &str, limits: StreamLimits) -> Result<Self, String> {
        Self::configured(limits).map_err(|error| format!("{setting}: {error}"))
    }

    /// The same bounds with a progress bound of `progress`, more than zero.
    pub fn with_progress(self, progress: Duration) -> Self {
        assert!(
            !progress.is_zero(),
            "a stream progress limit is more than zero"
        );
        Self { progress, ..self }
    }

    /// The progress bound.
    pub fn progress(self) -> Duration {
        self.progress
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
            Err(_elapsed) => Err(Idle::nothing(self.idle)),
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

/// The bytes that open an SSE event's data line (#2433): the field the
/// shared event rule ([`super::sse_common::event_data`]) reads.
const EVENT_LINE: &[u8] = super::sse_common::EVENT_FIELD.as_bytes();

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
///
/// An SSE body also has a progress bound (#2433): it is abandoned as one
/// that sends events but makes no progress when both hold — no event
/// carrying output ([`super::sse_progress`]) for the provider's progress
/// bound, and at least [`PROGRESS_EVENTS`] events without output, since the
/// last output (or the body's start). A body that is silent, or sends a
/// few events while it thinks, is the idle bound's alone.
#[derive(Debug)]
pub struct EventIdle {
    bound: Duration,
    deadline: tokio::time::Instant,
    /// `None` when every read is progress; otherwise where the body's
    /// current line stands.
    line: Option<Line>,
    /// The output progress of an SSE body; `None` for any other body.
    progress: Option<Progress>,
}

/// How far an SSE body's output has come (#2433).
#[derive(Debug)]
struct Progress {
    bound: Duration,
    /// When output is due: the bound from the last output, or the start.
    deadline: tokio::time::Instant,
    /// Since when output is awaited: the last output, or the start.
    since: tokio::time::Instant,
    /// Events without output since the last output: at
    /// [`PROGRESS_EVENTS`] the body may be abandoned for making none.
    events_since: u32,
    /// When the last of those arrived, if any did.
    last_event: Option<tokio::time::Instant>,
    /// The data of the event line being read, up to [`EVENT_DATA_CAP`].
    data: Vec<u8>,
    /// Whether that line ran past the cap: then the type its kept start
    /// names decides whether it carried output.
    long: bool,
}

/// The longest event data the progress bound reads to classify: past it,
/// an event is judged by the type its kept start names (#2433 review), so
/// classifying never holds a second copy of a long line.
const EVENT_DATA_CAP: usize = 64 * 1024;

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
        let now = tokio::time::Instant::now();
        let progress = line.map(|_| Progress {
            bound: bound.progress,
            deadline: now + bound.progress,
            since: now,
            events_since: 0,
            last_event: None,
            data: Vec::new(),
            long: false,
        });
        Self {
            bound: bound.idle,
            deadline: now + bound.idle,
            line,
            progress,
        }
    }

    /// When the next read is abandoned: the idle bound's deadline, or the
    /// progress bound's when it may fire and falls first.
    fn due(&self) -> tokio::time::Instant {
        self.fires().0
    }

    /// The bound that fires first, and when: the idle bound, unless the
    /// progress count rule or the backstop may fire and falls strictly
    /// earlier (on a tie, the idle bound names the expiry).
    fn fires(&self) -> (tokio::time::Instant, Fires) {
        let idle = (self.deadline, Fires::Idle);
        let Some(progress) = &self.progress else {
            return idle;
        };
        [
            progress.count_due().map(|at| (at, Fires::Count)),
            progress.backstop_due().map(|at| (at, Fires::Backstop)),
        ]
        .into_iter()
        .flatten()
        .fold(idle, |first, next| match next.0 < first.0 {
            true => next,
            false => first,
        })
    }

    /// Await one read of the body, abandoning it as [`Idle`] once the whole
    /// bound has passed since the body's last progress (or its start), and
    /// observe what it read.
    pub async fn next<B: AsRef<[u8]>, E>(
        &mut self,
        read: impl Future<Output = Result<Option<B>, E>>,
    ) -> Result<Result<Option<B>, E>, Idle> {
        let read = match tokio::time::timeout_at(self.due(), read).await {
            Ok(read) => read,
            Err(_elapsed) => {
                // Named by the bound that fired.
                let progress = self.progress.as_ref().map(|p| p.bound);
                return Err(match (self.fires().1, progress, &self.line) {
                    (Fires::Count, Some(bound), _) => Idle::no_output(bound),
                    (Fires::Backstop, Some(bound), _) => Idle::backstop(bound),
                    (_, _, Some(_)) => Idle::no_event(self.bound),
                    (_, _, None) => Idle::nothing(self.bound),
                });
            }
        };
        if let Ok(Some(bytes)) = &read {
            self.observe(bytes.as_ref());
        }
        Ok(read)
    }

    /// Bytes of the body arrived: an event restarts the idle bound, and an
    /// event carrying output the progress bound.
    fn observe(&mut self, bytes: &[u8]) {
        let now = tokio::time::Instant::now();
        let event = match (&mut self.line, &mut self.progress) {
            (Some(line), Some(progress)) => bytes.iter().fold(false, |event, &byte| {
                let before = *line;
                *line = line.after(byte);
                progress.read(before, *line, byte, now);
                event || *line == Line::Event
            }),
            _ => true,
        };
        if event {
            self.deadline = now + self.bound;
        }
    }
}

/// Which bound an expiry is named by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fires {
    Idle,
    Count,
    Backstop,
}

/// How many progress limits without output end a body that still sends
/// events, however few (#2433 review round 3): three, 900 s by default —
/// the admission attempt deadline of a typical setup — so a slow drip, or
/// a ping-only think, is bounded where no admission bounds it.
pub const PROGRESS_BACKSTOP_FACTOR: u32 = 3;

impl Progress {
    /// When the count rule ends the body: once [`PROGRESS_EVENTS`] events
    /// without output came, at the progress bound from the last output.
    fn count_due(&self) -> Option<tokio::time::Instant> {
        (self.events_since >= PROGRESS_EVENTS).then_some(self.deadline)
    }

    /// When the backstop ends the body: at [`PROGRESS_BACKSTOP_FACTOR`]
    /// progress bounds from the last output, provided an event without
    /// output came within the last progress bound of it — events still
    /// coming, however slowly. A body silent since is the idle bound's.
    fn backstop_due(&self) -> Option<tokio::time::Instant> {
        let backstop = self.since + self.bound * PROGRESS_BACKSTOP_FACTOR;
        self.last_event
            .filter(|last| *last + self.bound >= backstop)
            .map(|_| backstop)
    }

    /// `byte` of an SSE body moved its line from `before` to `after` at
    /// `now`: an event's data is kept, and judged when its line ends.
    fn read(&mut self, before: Line, after: Line, byte: u8, now: tokio::time::Instant) {
        match (before, after) {
            (Line::Event, Line::Event) if self.data.len() < EVENT_DATA_CAP => self.data.push(byte),
            (Line::Event, Line::Event) => self.long = true,
            (Line::Event, _) => {
                let output = match self.long {
                    true => super::sse_progress::long_event_carries_output(&self.data),
                    false => {
                        let data = String::from_utf8_lossy(&self.data);
                        super::sse_progress::carries_output(data.trim_end())
                    }
                };
                self.data.clear();
                self.long = false;
                match output {
                    true => {
                        self.deadline = now + self.bound;
                        self.since = now;
                        self.events_since = 0;
                        self.last_event = None;
                    }
                    false => {
                        self.events_since = self.events_since.saturating_add(1);
                        self.last_event = Some(now);
                    }
                }
            }
            _ => {}
        }
        debug_assert!(self.data.len() <= EVENT_DATA_CAP);
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

/// The provider sent nothing, or no event, for the bound it carries; the
/// request was abandoned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Idle {
    bound: Duration,
    missing: Missing,
}

/// What an idle exchange went without (#2433 review).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Missing {
    /// Anything at all: no response head, no bytes of an error body.
    Bytes,
    /// An event: an SSE body may have kept alive, but sent no event.
    Event,
    /// Output: an SSE body kept sending events, none carrying output.
    Output,
    /// Output, for the backstop: events kept coming, however few.
    OutputBackstop,
}

impl std::fmt::Display for Idle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (timeout, missing) = match self.missing {
            Missing::Bytes => (STREAM_IDLE_TIMEOUT, "nothing"),
            Missing::Event => (STREAM_IDLE_TIMEOUT, "no event"),
            Missing::Output => (STREAM_PROGRESS_TIMEOUT, "events but no output"),
            Missing::OutputBackstop => {
                return write!(
                    f,
                    "{STREAM_PROGRESS_TIMEOUT}the provider sent events but no output for {} \
                     (the progress backstop, {PROGRESS_BACKSTOP_FACTOR} times its {} limit); \
                     the request was abandoned",
                    Span(self.bound * PROGRESS_BACKSTOP_FACTOR),
                    Span(self.bound)
                );
            }
        };
        write!(
            f,
            "{timeout}the provider sent {missing} for {}; the request was abandoned",
            Span(self.bound)
        )
    }
}

impl Idle {
    /// Nothing at all came for `bound`.
    pub(crate) fn nothing(bound: Duration) -> Self {
        Self {
            bound,
            missing: Missing::Bytes,
        }
    }

    /// No event of an SSE body came for `bound`.
    pub(crate) fn no_event(bound: Duration) -> Self {
        Self {
            bound,
            missing: Missing::Event,
        }
    }

    /// Events but no output came for `bound` (#2433).
    pub(crate) fn no_output(bound: Duration) -> Self {
        Self {
            bound,
            missing: Missing::Output,
        }
    }

    /// Events, however few, but no output came for the backstop of a
    /// progress bound of `bound` (#2433 review round 3).
    pub(crate) fn backstop(bound: Duration) -> Self {
        Self {
            bound,
            missing: Missing::OutputBackstop,
        }
    }

    /// Whether events kept coming, but no output (#2433): the attempt made
    /// no progress, rather than going idle.
    pub fn is_no_output(self) -> bool {
        matches!(self.missing, Missing::Output | Missing::OutputBackstop)
    }

    /// What stands for an error body the provider stopped sending: the
    /// status before it still decides the error's class.
    pub fn body_marker(self) -> String {
        let name = STREAM_IDLE_TIMEOUT.trim_end_matches(": ");
        format!("(error body abandoned: {name} after {})", Span(self.bound))
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

#[cfg(test)]
#[path = "stream_progress_tests.rs"]
mod progress_tests;
