//! Worker event construction and processing for the coding runtime.
//!
//! Builds typed `EventPayload` variants from structured inputs and
//! enforces payload size limits. All functions are pure — no I/O.
//!
//! Secret redaction is handled by `infrastructure::logging::redact_api_keys()`
//! which should be called by the infrastructure layer before persisting events.

use crate::domain::coding_event::EventPayload;

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

/// Check whether a payload exceeds the 1 MiB size limit.
/// Returns `(json_value, is_oversized)`. When oversized, sets
/// `truncated: true` on the returned value as a marker.
pub fn is_payload_oversized(payload: &EventPayload) -> (serde_json::Value, bool) {
    let value = payload_to_json(payload);
    let serialized = serde_json::to_string(&value).unwrap_or_default();
    if serialized.len() > MAX_PAYLOAD_BYTES {
        let mut marked = value;
        if let Some(obj) = marked.as_object_mut() {
            obj.insert("truncated".to_string(), serde_json::Value::Bool(true));
        }
        (marked, true)
    } else {
        (value, false)
    }
}

/// Convert an `EventPayload` to a JSON value (without the `type` tag).
///
/// The internally-tagged serde representation includes a `"type"` field
/// that duplicates the envelope's `event_type`. This strips it so the
/// payload can be embedded in an `EventEnvelope` cleanly.
pub fn payload_to_json(payload: &EventPayload) -> serde_json::Value {
    let mut value = serde_json::to_value(payload).unwrap_or_default();
    if let Some(obj) = value.as_object_mut() {
        obj.remove("type");
    }
    value
}

#[cfg(test)]
#[path = "coding_worker_events_tests.rs"]
mod tests;
