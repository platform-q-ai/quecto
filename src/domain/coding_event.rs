use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::coding_job::{CancelInitiator, CancelReason, ErrorCode, JobState};

/// Source of an event in the coding runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventSource {
    MainAgent,
    Coordinator,
    Worker,
    ChildAgent,
}

impl std::fmt::Display for EventSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::MainAgent => "main_agent",
            Self::Coordinator => "coordinator",
            Self::Worker => "worker",
            Self::ChildAgent => "child_agent",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for EventSource {
    type Err = EventSourceParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "main_agent" => Ok(Self::MainAgent),
            "coordinator" => Ok(Self::Coordinator),
            "worker" => Ok(Self::Worker),
            "child_agent" => Ok(Self::ChildAgent),
            _ => Err(EventSourceParseError(s.to_string())),
        }
    }
}

#[derive(Debug, Error)]
#[error("invalid event source: {0}")]
pub struct EventSourceParseError(String);

/// JSONL event envelope — the outer wrapper for all coding runtime events.
///
/// Every event line in the log conforms to this shape. The `payload` field
/// contains the event-type-specific data (see `EventPayload`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    /// Contract version (pattern: `^1\.[0-9]+$`).
    pub v: String,
    /// ISO 8601 timestamp.
    pub ts: String,
    /// Run identifier.
    pub run_id: String,
    /// Job identifier.
    pub job_id: String,
    /// Who emitted this event.
    pub source: EventSource,
    /// Event type string (e.g. "job.start", "tool.result").
    #[serde(rename = "type")]
    pub event_type: String,
    /// Monotonically increasing sequence number, scoped per (source, job_id).
    pub seq: u64,
    /// Event-type-specific payload.
    pub payload: serde_json::Value,
}

/// All 22 event payload types defined by the contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EventPayload {
    // --- Job lifecycle (7) ---
    #[serde(rename = "job.start")]
    JobStart {
        goal: String,
        base_ref: String,
        branch: String,
    },

    #[serde(rename = "job.ready")]
    JobReady {
        worker_pid: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        clone_duration_ms: Option<u64>,
    },

    #[serde(rename = "job.status")]
    JobStatus {
        state: JobState,
        summary: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        progress: Option<u32>,
    },

    #[serde(rename = "job.blocked")]
    JobBlocked {
        reason: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        needs: Option<String>,
    },

    #[serde(rename = "job.resumed")]
    JobResumed { reason: String },

    #[serde(rename = "job.cancel")]
    JobCancel {
        reason: CancelReason,
        #[serde(skip_serializing_if = "Option::is_none")]
        initiated_by: Option<CancelInitiator>,
    },

    #[serde(rename = "job.end")]
    JobEnd {
        state: JobState,
        summary: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_code: Option<ErrorCode>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error_detail: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_retriable: Option<bool>,
        #[serde(skip_serializing_if = "Option::is_none")]
        artifacts: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
    },

    // --- Todo tracking (4) ---
    #[serde(rename = "todo.create")]
    TodoCreate {
        todo_id: String,
        title: String,
        status: String, // always "pending" on creation
        #[serde(skip_serializing_if = "Option::is_none")]
        owner: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        depends_on: Option<Vec<String>>,
    },

    #[serde(rename = "todo.update")]
    TodoUpdate {
        todo_id: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },

    #[serde(rename = "todo.blocked")]
    TodoBlocked {
        todo_id: String,
        reason: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        needs: Option<String>,
    },

    #[serde(rename = "todo.complete")]
    TodoComplete {
        todo_id: String,
        result: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        artifact_refs: Option<Vec<String>>,
    },

    // --- Tool execution (2) ---
    #[serde(rename = "tool.start")]
    ToolStart {
        tool: String,
        call_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        args_preview: Option<String>,
    },

    #[serde(rename = "tool.result")]
    ToolResult {
        tool: String,
        call_id: String,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        diff_ref: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        stderr_ref: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        stdout_ref: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        truncated: Option<bool>,
    },

    // --- Child agents (3) ---
    #[serde(rename = "spawn.request")]
    SpawnRequest {
        request_id: String,
        agent_type: String,
        scope: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        expected_output: Option<String>,
    },

    #[serde(rename = "spawn.decision")]
    SpawnDecision {
        request_id: String,
        approved: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },

    #[serde(rename = "spawn.result")]
    SpawnResult {
        request_id: String,
        state: JobState,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        artifact_refs: Option<Vec<String>>,
    },

    // --- Skills (2) ---
    #[serde(rename = "skills.applied")]
    SkillsApplied {
        skills: Vec<String>,
        snapshot_ref: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        profile: Option<String>,
    },

    #[serde(rename = "skills.suggested")]
    SkillsSuggested {
        skills: Vec<String>,
        reason: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        by: Option<String>,
    },

    // --- Publish (2) ---
    #[serde(rename = "publish.request")]
    PublishRequest {
        action: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        base: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        head: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        body: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        labels: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reviewers: Option<Vec<String>>,
    },

    #[serde(rename = "publish.result")]
    PublishResult {
        action: String,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        pr_number: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    // --- Artifacts and logging (2) ---
    #[serde(rename = "artifact.created")]
    ArtifactCreated {
        artifact_id: String,
        artifact_type: String,
        path: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        size_bytes: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },

    #[serde(rename = "log.message")]
    LogMessage {
        level: String,
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        context: Option<serde_json::Value>,
    },
}

/// Known event type strings (22 total).
pub const KNOWN_EVENT_TYPES: &[&str] = &[
    "job.start",
    "job.ready",
    "job.status",
    "job.blocked",
    "job.resumed",
    "job.cancel",
    "job.end",
    "todo.create",
    "todo.update",
    "todo.blocked",
    "todo.complete",
    "tool.start",
    "tool.result",
    "spawn.request",
    "spawn.decision",
    "spawn.result",
    "skills.applied",
    "skills.suggested",
    "publish.request",
    "publish.result",
    "artifact.created",
    "log.message",
];

/// Check if an event type is known.
pub fn is_known_event_type(event_type: &str) -> bool {
    KNOWN_EVENT_TYPES.contains(&event_type)
}

/// Check if a version string matches the contract pattern `^1\.[0-9]+$`.
pub fn is_compatible_version(v: &str) -> bool {
    if let Some(rest) = v.strip_prefix("1.") {
        !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit())
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_source_display_round_trip() {
        for source in [
            EventSource::MainAgent,
            EventSource::Coordinator,
            EventSource::Worker,
            EventSource::ChildAgent,
        ] {
            let s = source.to_string();
            let parsed: EventSource = s.parse().unwrap();
            assert_eq!(source, parsed);
        }
    }

    #[test]
    fn test_known_event_types_count() {
        assert_eq!(KNOWN_EVENT_TYPES.len(), 22);
    }

    #[test]
    fn test_is_known_event_type() {
        assert!(is_known_event_type("job.start"));
        assert!(is_known_event_type("log.message"));
        assert!(!is_known_event_type("unknown.future_event"));
    }

    #[test]
    fn test_is_compatible_version() {
        assert!(is_compatible_version("1.0"));
        assert!(is_compatible_version("1.3"));
        assert!(is_compatible_version("1.99"));
        assert!(!is_compatible_version("2.0"));
        assert!(!is_compatible_version("0.1"));
        assert!(!is_compatible_version("1."));
        assert!(!is_compatible_version("abc"));
    }
}
