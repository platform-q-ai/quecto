//! Bounded transport facts. No provider messages or response content belong here.
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AttemptDiagnostics {
    pub attempt_number: u32,
    pub started_unix_ms: u64,
    pub finished_unix_ms: u64,
    pub elapsed_ms: u64,
    pub wire_status: Option<u16>,
    pub headers: Vec<SafeHeader>,
    pub stop_reason: Option<TerminalStopReason>,
    pub terminal_event: Option<TerminalEvent>,
    pub error_code: Option<ErrorCode>,
    pub incomplete_reason: Option<IncompleteReason>,
    pub event_count: u32,
    pub generated_text: bool,
    pub generated_tool_call: bool,
    pub generated_thinking: bool,
    /// When the first text, thinking or tool-call delta arrived, from the
    /// attempt's start (#2151); `None` before any did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_token_ms: Option<u64>,
    /// Bytes of output the attempt streamed: text, thinking, refusal and
    /// tool-call argument deltas, never the events around them (#2210).
    #[serde(default)]
    pub output_bytes: u64,
    pub oversized_lines: u32,
    pub parse_errors: u32,
    pub unknown_events: u32,
    pub termination: Termination,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SafeHeader {
    pub name: HeaderName,
    pub value: HeaderValue,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HeaderValue {
    Sha256(String),
    Number(u64),
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum HeaderName {
    RequestId,
    XRequestId,
    RetryAfter,
    RetryAfterMs,
    RemainingRequests,
    RemainingTokens,
    LimitRequests,
    LimitTokens,
    ResetRequests,
    ResetTokens,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TerminalEvent {
    Done,
    ResponseCompleted,
    ResponseFailed,
    ResponseIncomplete,
    Error,
    MessageStop,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ErrorCode {
    RateLimitExceeded,
    Overloaded,
    ServerError,
    InsufficientQuota,
    UsageLimitReached,
    AuthenticationError,
    PermissionError,
    InvalidRequestError,
    InvalidApiKey,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum IncompleteReason {
    MaxOutputTokens,
    ContentFilter,
}
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Termination {
    #[default]
    Dropped,
    Completed,
    /// The body ended with no terminal event and nothing said how. No
    /// longer recorded (#2249 review: such a body is `CutShort`); kept so
    /// older records still read.
    Eof,
    HttpError,
    ReadError,
    SendError,
    Deadline,
    Cancelled,
    ReceiverClosed,
    /// The harness refused the reply: a limit it enforces (a line, content
    /// or tool-call arguments over their size), or a reply it could not
    /// parse or accept (#2156 review).
    Rejected,
    /// The provider sent no response head, or no SSE event, for the stream
    /// idle limit, and the harness abandoned the attempt (#2210). Bytes may
    /// have arrived: keep-alives (SSE comments, blank lines) are no event,
    /// so a reply held open by them alone ends here too (#2433).
    Idle,
    /// A whole non-streaming reply did not arrive within the total reply
    /// bound, and the harness abandoned the attempt (#2210 review).
    TimedOut,
    /// The reply streamed more output than the attempt's output cap, and the
    /// harness abandoned the attempt as a runaway (#2210).
    OutputCapped,
    /// The request ended — a run deadline, an abort or steer, a shutdown —
    /// while this attempt was still in flight; recorded from what it had
    /// streamed by then (#2210).
    Interrupted,
    /// The body ended before the protocol's terminal event (`[DONE]`,
    /// `response.completed`, `message_stop`, or an OpenAI `finish_reason`),
    /// or with no event at all: a reply cut short in transport, never taken
    /// as a whole one, streamed or read whole, with admission or without
    /// (#2249 review).
    CutShort,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TerminalStopReason {
    EndTurn,
    MaxTokens,
    ToolUse,
    Refusal,
    Error,
    Aborted,
    Unknown,
}
