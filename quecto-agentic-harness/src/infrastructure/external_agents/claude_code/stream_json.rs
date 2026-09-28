//! Decode one line of `claude -p --output-format stream-json` into the
//! domain's [`ExternalAgentEvent`]s (#2285).
//!
//! One line can carry several events (an assistant line's content blocks
//! and its `error`, a user line's tool results), so a line decodes to a
//! list. Every field another CLI version may omit is optional here. An
//! event type this codec does not know becomes
//! [`ExternalAgentEvent::Unknown`] and is logged; nothing panics. A line
//! that is not a JSON object, or a known event whose fields have the
//! wrong types, is a [`StreamJsonError`].

use crate::domain::external_agent::stream::{
    AssistantContent, BackgroundTask, ExternalAgentEvent, InitEvent, McpServerStatus, ModelUsage,
    PermissionDenial, RateLimitInfo, RateLimitStatus, RateLimitWindow, ResultEvent,
    TaskNotification, TaskStarted, TokenCounts, ToolResultEvent,
};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StreamJsonError {
    #[error("stream-json line is not JSON: {0}")]
    NotJson(String),
    #[error("stream-json line is not a JSON object")]
    NotAnObject,
    #[error("stream-json `{kind}` event is malformed: {reason}")]
    Malformed { kind: String, reason: String },
}

/// Decode one NDJSON line.
pub fn decode_line(line: &str) -> Result<Vec<ExternalAgentEvent>, StreamJsonError> {
    // RED (#2285): not implemented yet.
    let _ = line;
    Ok(vec![ExternalAgentEvent::Unknown {
        kind: String::new(),
    }])
}

#[cfg(test)]
#[path = "stream_json_tests.rs"]
mod tests;
