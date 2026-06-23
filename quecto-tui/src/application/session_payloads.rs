//! Typed application values for TUI session-related wire payloads.
//!
//! The infrastructure client still receives raw JSON from the UDS protocol, but
//! interface code should not hand-parse those protocol shapes in render/app
//! paths. These mappers keep that translation in the application layer.

/// Parsed session statistics used by chat status lines and footer indicators.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionStats {
    pub session_key: String,
    pub total_messages: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost: f64,
    pub context_usage: Option<(u64, usize)>,
}

/// A persisted session entry suitable for the resume selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeSessionSummary {
    pub key: String,
    pub title: String,
    pub message_count: u64,
    pub updated_unix_secs: Option<u64>,
}

/// Displayable chat messages from a resumed session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumedChatMessage {
    User(String),
    Assistant(String),
}

/// Parse a `get_session_stats` response payload into a typed value with the
/// same forgiving defaults the TUI historically used.
pub fn parse_session_stats(data: &serde_json::Value) -> SessionStats {
    let context_tokens = data.get("contextTokens").and_then(|v| v.as_u64());
    let max_context_tokens = data
        .get("maxContextTokens")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize);

    SessionStats {
        session_key: data
            .get("sessionKey")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string(),
        total_messages: data
            .get("totalMessages")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        input_tokens: data
            .get("tokens")
            .and_then(|t| t.get("input"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        output_tokens: data
            .get("tokens")
            .and_then(|t| t.get("output"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        cost: data.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.0),
        context_usage: context_tokens.zip(max_context_tokens),
    }
}

/// Parse a `list_sessions` response payload into selector summaries. Entries
/// without a human-readable title/name are skipped because they cannot be shown
/// or selected meaningfully.
pub fn parse_resume_sessions(data: &serde_json::Value) -> Vec<ResumeSessionSummary> {
    session_values(data)
        .iter()
        .filter_map(|session| {
            let title = session
                .get("title")
                .or_else(|| session.get("name"))
                .and_then(|v| v.as_str())?;
            let key = session.get("key").and_then(|v| v.as_str()).unwrap_or(title);
            Some(ResumeSessionSummary {
                key: key.to_string(),
                title: title.to_string(),
                message_count: session
                    .get("messageCount")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                updated_unix_secs: session
                    .get("updatedUnixSecs")
                    .or_else(|| session.get("updatedAt"))
                    .and_then(|v| v.as_u64()),
            })
        })
        .collect()
}

/// Parse a `get_messages` payload after session resume into displayable chat
/// messages. Unknown roles and empty assistant messages are intentionally
/// omitted to preserve previous TUI behavior.
pub fn parse_resumed_messages(data: &serde_json::Value) -> Vec<ResumedChatMessage> {
    message_values(data)
        .iter()
        .filter_map(|message| {
            let role = message.get("role").and_then(|v| v.as_str()).unwrap_or("");
            let content = message
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            match role {
                "user" => Some(ResumedChatMessage::User(content)),
                "assistant" if !content.is_empty() => Some(ResumedChatMessage::Assistant(content)),
                _ => None,
            }
        })
        .collect()
}

/// Whether the payload explicitly contained session entries, even if none are
/// resumable after parsing. Allows the interface to keep its more specific
/// empty-vs-malformed user messages without parsing raw fields itself.
pub fn has_session_entries(data: &serde_json::Value) -> bool {
    !session_values(data).is_empty()
}

fn session_values(data: &serde_json::Value) -> &[serde_json::Value] {
    data.get("sessions")
        .and_then(|v| v.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn message_values(data: &serde_json::Value) -> &[serde_json::Value] {
    data.get("messages")
        .and_then(|v| v.as_array())
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

#[cfg(test)]
#[path = "session_payloads_tests.rs"]
mod tests;
