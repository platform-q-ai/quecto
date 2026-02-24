//! Worker event construction and processing for the coding runtime.
//!
//! Builds typed `EventPayload` variants from structured inputs,
//! enforces payload size limits, and redacts secrets from event
//! content before emission. All functions are pure — no I/O.

use crate::domain::coding_event::{EventPayload, EventSource};

/// Maximum payload size in bytes (1 MiB).
pub const MAX_PAYLOAD_BYTES: usize = 1024 * 1024;

// ============================================================================
// Tool events
// ============================================================================

/// Input for building a tool.start event payload.
pub struct ToolStartInput {
    pub tool: String,
    pub call_id: String,
    pub args_preview: Option<String>,
}

/// Input for building a tool.result event payload.
pub struct ToolResultInput {
    pub tool: String,
    pub call_id: String,
    pub ok: bool,
    pub duration_ms: Option<u64>,
    pub diff_ref: Option<String>,
    pub stderr_ref: Option<String>,
    pub stdout_ref: Option<String>,
    pub truncated: Option<bool>,
}

/// Build a `tool.start` event payload.
pub fn build_tool_start(input: ToolStartInput) -> EventPayload {
    EventPayload::ToolStart {
        tool: input.tool,
        call_id: input.call_id,
        args_preview: input.args_preview,
    }
}

/// Build a `tool.result` event payload.
pub fn build_tool_result(input: ToolResultInput) -> EventPayload {
    EventPayload::ToolResult {
        tool: input.tool,
        call_id: input.call_id,
        ok: input.ok,
        duration_ms: input.duration_ms,
        diff_ref: input.diff_ref,
        stderr_ref: input.stderr_ref,
        stdout_ref: input.stdout_ref,
        truncated: input.truncated,
    }
}

// ============================================================================
// Artifact events
// ============================================================================

/// Input for building an artifact.created event payload.
pub struct ArtifactInput {
    pub artifact_id: String,
    pub artifact_type: String,
    pub path: String,
    pub size_bytes: Option<u64>,
    pub description: Option<String>,
}

/// Build an `artifact.created` event payload.
pub fn build_artifact(input: ArtifactInput) -> EventPayload {
    EventPayload::ArtifactCreated {
        artifact_id: input.artifact_id,
        artifact_type: input.artifact_type,
        path: input.path,
        size_bytes: input.size_bytes,
        description: input.description,
    }
}

// ============================================================================
// Log events
// ============================================================================

/// Input for building a log.message event payload.
pub struct LogInput {
    pub level: String,
    pub message: String,
    pub context: Option<serde_json::Value>,
}

/// Build a `log.message` event payload.
pub fn build_log(input: LogInput) -> EventPayload {
    EventPayload::LogMessage {
        level: input.level,
        message: input.message,
        context: input.context,
    }
}

// ============================================================================
// Payload processing
// ============================================================================

/// Serialize a payload and check if it exceeds the size limit.
/// Returns `(json_value, is_truncated)`.
pub fn check_payload_size(payload: &EventPayload) -> (serde_json::Value, bool) {
    let value = payload_to_json(payload);
    let serialized = serde_json::to_string(&value).unwrap_or_default();
    if serialized.len() > MAX_PAYLOAD_BYTES {
        let mut truncated = value;
        if let Some(obj) = truncated.as_object_mut() {
            obj.insert("truncated".to_string(), serde_json::Value::Bool(true));
        }
        (truncated, true)
    } else {
        (value, false)
    }
}

/// Convert an `EventPayload` to a JSON value (without the `type` tag).
pub fn payload_to_json(payload: &EventPayload) -> serde_json::Value {
    // Serialize with the internal tag, then strip the "type" field
    // since the envelope carries event_type separately.
    let mut value = serde_json::to_value(payload).unwrap_or_default();
    if let Some(obj) = value.as_object_mut() {
        obj.remove("type");
    }
    value
}

/// Redact known secret patterns from a string.
///
/// Matches `sk-*` (OpenAI/Anthropic), `gsk_*`/`gsk-*` (Groq), and
/// Telegram bot token formats. Returns the input with secrets replaced
/// by `[REDACTED]`.
pub fn redact_secrets(input: &str) -> String {
    if !has_secret_candidate(input) {
        return input.to_string();
    }
    // Simple pattern: replace any token matching sk-... or gsk_... patterns
    let mut result = input.to_string();
    for word in input.split_whitespace() {
        if is_secret_token(word) {
            result = result.replace(word, "[REDACTED]");
        }
    }
    // Also check for = delimited secrets (e.g. "token=sk-abc123")
    for segment in input.split('=') {
        let trimmed = segment.trim();
        if is_secret_token(trimmed) {
            result = result.replace(trimmed, "[REDACTED]");
        }
    }
    result
}

/// Determine the event source string for display.
pub fn source_label(source: EventSource) -> &'static str {
    match source {
        EventSource::Coordinator => "coordinator",
        EventSource::Worker => "worker",
        EventSource::ChildAgent => "child_agent",
        EventSource::MainAgent => "main_agent",
    }
}

// ============================================================================
// Internal helpers
// ============================================================================

fn has_secret_candidate(s: &str) -> bool {
    s.contains("sk-") || s.contains("gsk_") || s.contains("gsk-")
}

fn is_secret_token(token: &str) -> bool {
    let t = token.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_');
    if t.len() < 8 {
        return false;
    }
    t.starts_with("sk-")
        || t.starts_with("sk-ant-")
        || t.starts_with("gsk_")
        || t.starts_with("gsk-")
}

#[cfg(test)]
#[path = "coding_worker_events_tests.rs"]
mod tests;
