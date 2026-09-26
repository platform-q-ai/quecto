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
    Eof,
    HttpError,
    ReadError,
    SendError,
    Deadline,
    Cancelled,
    ReceiverClosed,
    /// The harness refused the reply part way: a limit it enforces, or a
    /// reply it could not accept (#2156 review).
    Rejected,
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
