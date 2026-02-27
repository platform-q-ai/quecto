//! Domain types for coordinator file-based IPC.
//!
//! The main agent communicates with the coordinator subagent via JSON files
//! in an inbox/outbox directory structure. These types define the wire format
//! for commands, responses, notifications, and state snapshots.

use serde::{Deserialize, Serialize};

// ============================================================================
// IPC Command (main agent -> coordinator)
// ============================================================================

/// A command written by the main agent to `coordinator/inbox/<command_id>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatorIpcCommand {
    /// Unique identifier for this command (used as filename and for response correlation).
    pub command_id: String,
    /// The action to perform (e.g. "run", "status", "cancel", "cleanup", "list", "shutdown").
    pub action: String,
    /// Action-specific payload (same shape as the existing coding_job tool JSON).
    #[serde(default)]
    pub payload: serde_json::Value,
}

// ============================================================================
// IPC Response (coordinator -> main agent)
// ============================================================================

/// A response written by the coordinator to `coordinator/outbox/<command_id>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatorIpcResponse {
    /// The command_id this response correlates to.
    pub command_id: String,
    /// Whether the command succeeded.
    pub ok: bool,
    /// Response body (action-specific JSON).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
    /// Error description on failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ============================================================================
// Notifications (coordinator -> main agent, proactive)
// ============================================================================

/// Notification types emitted proactively by the coordinator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationType {
    /// Worker asks a question or needs human input.
    WorkerBlocked,
    /// Unexpected crash, timeout, or resource limit hit.
    JobFailed,
    /// No progress for N minutes.
    WorkerStuck,
    /// All jobs in a batch finished.
    BatchComplete,
    /// Worker attempted forbidden action.
    PolicyViolation,
}

impl std::fmt::Display for NotificationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::WorkerBlocked => "worker_blocked",
            Self::JobFailed => "job_failed",
            Self::WorkerStuck => "worker_stuck",
            Self::BatchComplete => "batch_complete",
            Self::PolicyViolation => "policy_violation",
        };
        f.write_str(s)
    }
}

/// A proactive notification written to `coordinator/notifications/<ts>_<type>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatorNotification {
    /// Notification type.
    #[serde(rename = "type")]
    pub notification_type: NotificationType,
    /// Related job ID (None for batch-level notifications).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    /// Related job IDs (for batch_complete).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub job_ids: Vec<String>,
    /// Human-readable detail or question.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Minutes without progress (for worker_stuck).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub no_progress_minutes: Option<u32>,
    /// ISO 8601 timestamp.
    pub ts: String,
}

// ============================================================================
// Coordinator State Snapshot
// ============================================================================

/// Periodic state snapshot written to `coordinator/state.json`.
///
/// The main agent reads this for fast-path status queries without
/// going through the inbox/outbox round-trip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinatorState {
    /// Whether the coordinator considers itself alive.
    pub alive: bool,
    /// Number of active (non-terminal) jobs.
    pub active_jobs: u32,
    /// ISO 8601 timestamp of the last heartbeat write.
    pub last_heartbeat: String,
    /// Summary of job states (e.g. {"running": 2, "queued": 1}).
    #[serde(default)]
    pub job_summary: serde_json::Value,
}

// ============================================================================
// IPC Port Trait
// ============================================================================

/// Port for coordinator file-based IPC operations.
///
/// The main agent side uses this to write commands and read responses.
/// The coordinator side uses this to read commands and write responses.
/// Both sides share notification and state operations.
pub trait CoordinatorIpc: Send + Sync {
    /// Write a command to the inbox.
    fn write_command(&self, cmd: &CoordinatorIpcCommand) -> Result<(), String>;

    /// Read all pending commands from the inbox.
    fn read_pending_commands(&self) -> Result<Vec<CoordinatorIpcCommand>, String>;

    /// Acknowledge (remove) a processed inbox command.
    fn acknowledge_command(&self, command_id: &str) -> Result<(), String>;

    /// Write a response to the outbox.
    fn write_response(&self, resp: &CoordinatorIpcResponse) -> Result<(), String>;

    /// Poll the outbox for a response to a specific command_id.
    /// Returns `None` if no response file exists yet.
    fn read_response(&self, command_id: &str) -> Result<Option<CoordinatorIpcResponse>, String>;

    /// Write a notification to the notifications directory.
    fn write_notification(&self, notif: &CoordinatorNotification) -> Result<(), String>;

    /// Read all pending notifications, ordered by timestamp.
    fn read_notifications(&self) -> Result<Vec<CoordinatorNotification>, String>;

    /// Acknowledge (remove) a notification file.
    fn acknowledge_notification(&self, filename: &str) -> Result<(), String>;

    /// Write the coordinator state snapshot.
    fn write_state(&self, state: &CoordinatorState) -> Result<(), String>;

    /// Read the coordinator state snapshot. Returns `None` if not yet written.
    fn read_state(&self) -> Result<Option<CoordinatorState>, String>;

    /// Write the coordinator PID.
    fn write_pid(&self, pid: u32) -> Result<(), String>;

    /// Read the coordinator PID. Returns `None` if pid file doesn't exist.
    fn read_pid(&self) -> Result<Option<u32>, String>;

    /// Check if the coordinator process is alive (PID exists and process is running).
    fn is_coordinator_alive(&self) -> bool;
}

// ============================================================================
// Coordinator Spawner Port
// ============================================================================

/// Result of an `ensure_alive` call on the coordinator spawner.
#[derive(Debug, Clone)]
pub struct SpawnResult {
    /// PID of the coordinator process (existing or newly spawned).
    pub pid: u32,
    /// Whether the coordinator was freshly spawned (true) or was already running (false).
    pub spawned: bool,
}

/// Port for spawning and ensuring liveness of the coordinator subagent process.
///
/// The main agent calls `ensure_alive()` before each IPC dispatch. If the
/// coordinator is not running, the spawner launches a new `quecto agent`
/// child process with a coordinator-specific system prompt and records its PID.
pub trait CoordinatorSpawner: Send + Sync {
    /// Ensure the coordinator process is alive. If it is not, spawn a new one.
    ///
    /// Returns the PID of the coordinator (existing or newly spawned).
    fn ensure_alive(&self) -> Result<SpawnResult, String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_command_round_trip() {
        let cmd = CoordinatorIpcCommand {
            command_id: "cmd_001".to_string(),
            action: "run".to_string(),
            payload: serde_json::json!({"goal": "Fix bug"}),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let parsed: CoordinatorIpcCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.command_id, "cmd_001");
        assert_eq!(parsed.action, "run");
    }

    #[test]
    fn test_response_ok_round_trip() {
        let resp = CoordinatorIpcResponse {
            command_id: "cmd_001".to_string(),
            ok: true,
            body: Some(serde_json::json!({"job_id": "j1"})),
            error: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: CoordinatorIpcResponse = serde_json::from_str(&json).unwrap();
        assert!(parsed.ok);
        assert!(parsed.error.is_none());
    }

    #[test]
    fn test_response_error_round_trip() {
        let resp = CoordinatorIpcResponse {
            command_id: "cmd_002".to_string(),
            ok: false,
            body: None,
            error: Some("not_found".to_string()),
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: CoordinatorIpcResponse = serde_json::from_str(&json).unwrap();
        assert!(!parsed.ok);
        assert_eq!(parsed.error.as_deref(), Some("not_found"));
    }

    #[test]
    fn test_notification_type_display() {
        assert_eq!(
            NotificationType::WorkerBlocked.to_string(),
            "worker_blocked"
        );
        assert_eq!(NotificationType::JobFailed.to_string(), "job_failed");
        assert_eq!(NotificationType::WorkerStuck.to_string(), "worker_stuck");
        assert_eq!(
            NotificationType::BatchComplete.to_string(),
            "batch_complete"
        );
        assert_eq!(
            NotificationType::PolicyViolation.to_string(),
            "policy_violation"
        );
    }

    #[test]
    fn test_notification_round_trip() {
        let notif = CoordinatorNotification {
            notification_type: NotificationType::JobFailed,
            job_id: Some("job_001".to_string()),
            job_ids: vec![],
            detail: Some("OOM killed".to_string()),
            no_progress_minutes: None,
            ts: "2026-01-15T10:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&notif).unwrap();
        let parsed: CoordinatorNotification = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.notification_type, NotificationType::JobFailed);
        assert_eq!(parsed.job_id.as_deref(), Some("job_001"));
    }

    #[test]
    fn test_notification_batch_complete() {
        let notif = CoordinatorNotification {
            notification_type: NotificationType::BatchComplete,
            job_id: None,
            job_ids: vec!["j1".to_string(), "j2".to_string(), "j3".to_string()],
            detail: Some("3 succeeded".to_string()),
            no_progress_minutes: None,
            ts: "2026-01-15T10:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&notif).unwrap();
        let parsed: CoordinatorNotification = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.job_ids.len(), 3);
    }

    #[test]
    fn test_notification_worker_stuck() {
        let notif = CoordinatorNotification {
            notification_type: NotificationType::WorkerStuck,
            job_id: Some("job_020".to_string()),
            job_ids: vec![],
            detail: None,
            no_progress_minutes: Some(30),
            ts: "2026-01-15T10:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&notif).unwrap();
        let parsed: CoordinatorNotification = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.no_progress_minutes, Some(30));
    }

    #[test]
    fn test_state_round_trip() {
        let state = CoordinatorState {
            alive: true,
            active_jobs: 2,
            last_heartbeat: "2026-01-15T10:00:00Z".to_string(),
            job_summary: serde_json::json!({"running": 1, "queued": 1}),
        };
        let json = serde_json::to_string(&state).unwrap();
        let parsed: CoordinatorState = serde_json::from_str(&json).unwrap();
        assert!(parsed.alive);
        assert_eq!(parsed.active_jobs, 2);
    }

    #[test]
    fn test_command_default_payload() {
        let json = r#"{"command_id":"c1","action":"list"}"#;
        let cmd: CoordinatorIpcCommand = serde_json::from_str(json).unwrap();
        assert!(cmd.payload.is_null());
    }

    #[test]
    fn test_notification_type_serde_round_trip() {
        for nt in [
            NotificationType::WorkerBlocked,
            NotificationType::JobFailed,
            NotificationType::WorkerStuck,
            NotificationType::BatchComplete,
            NotificationType::PolicyViolation,
        ] {
            let json = serde_json::to_string(&nt).unwrap();
            let parsed: NotificationType = serde_json::from_str(&json).unwrap();
            assert_eq!(nt, parsed);
        }
    }
}
