// Shared subagent registry types for spawn + agent_cmd tools (#421).
// Extended with live status tracking for persistent monitor (#522).
// Extended with await signaling for agent_cmd await (#612).

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::domain::workflow::{VerdictStatus, WorkflowMode};

/// Live status of a spawned subagent, updated by the monitor task (#522).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SubagentStatus {
    /// Child process spawned but not yet confirmed running.
    #[default]
    Starting,
    /// Agent finished processing and is waiting for the next prompt.
    Idle,
    /// Agent is actively processing a prompt or executing a tool.
    Running,
    /// Last tool execution returned an error.
    Error,
    /// Child process exited (connection closed or process reaped).
    Exited,
}

impl SubagentStatus {
    /// Wire-format string for the UDS protocol (lowercase, zero-alloc).
    pub fn to_wire_str(&self) -> &'static str {
        match self {
            Self::Starting => "starting",
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Error => "error",
            Self::Exited => "exited",
        }
    }
}

impl fmt::Display for SubagentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Starting => write!(f, "Starting"),
            Self::Idle => write!(f, "Idle"),
            Self::Running => write!(f, "Running"),
            Self::Error => write!(f, "Error"),
            Self::Exited => write!(f, "Exited"),
        }
    }
}

/// Entry for a spawned subagent in the shared registry.
#[derive(Debug, Clone)]
pub struct SubagentEntry {
    /// Path to the child's UDS socket.
    pub socket_path: PathBuf,
    /// Child process PID (0 in stub mode).
    pub pid: u32,
    /// Live status updated by the monitor task (#522).
    pub status: SubagentStatus,
    /// Name of the last tool being executed (from tool_execution_start).
    pub last_tool: Option<String>,
    /// Description of the last error (from tool_execution_end with is_error or agent_error).
    pub last_error: Option<String>,
    /// Run-level agent error (for example provider/model failure). Unlike a tool
    /// error, this means the prompt run failed and `agent_cmd await` should
    /// return a structured error instead of waiting for recovery.
    pub run_error: Option<String>,
    /// When this entry was last updated by the monitor.
    pub updated_at: Instant,
    /// Abort handle for the monitor task (if running).
    pub monitor_handle: Option<Arc<tokio::task::JoinHandle<()>>>,
    /// Monotonic notification id for this subagent.
    pub notification_sequence: u64,
    /// Exit signal sender — the reaper task sends the exit code/signal through
    /// this channel so that a waiting `await` call can return immediately (#612).
    pub exit_signal_tx: Option<ExitSignalTx>,
    /// The spawning agent's id, for reconstructing the unit tree (PRD Stage B).
    pub parent_id: Option<String>,
    /// Latest workflow snapshot reported by the child's monitor (PRD Stage B).
    pub workflow: Option<WorkflowSnapshot>,
}

impl SubagentEntry {
    /// Create a new entry with `Starting` status.
    pub fn new(socket_path: PathBuf, pid: u32) -> Self {
        Self {
            socket_path,
            pid,
            status: SubagentStatus::Starting,
            last_tool: None,
            last_error: None,
            run_error: None,
            updated_at: Instant::now(),
            monitor_handle: None,
            notification_sequence: 0,
            exit_signal_tx: None,
            parent_id: None,
            workflow: None,
        }
    }
}

/// Shared registry of spawned subagents (agent_id → entry).
pub type SubagentRegistry = Arc<Mutex<HashMap<String, SubagentEntry>>>;

/// Create a new empty registry.
pub fn new_registry() -> SubagentRegistry {
    Arc::new(Mutex::new(HashMap::new()))
}

// ─── Await support (#612) ────────────────────────────────────────────────────

/// Result of an `agent_cmd await` call.
///
/// `status`/`reason` describe the await lifecycle (idle/exited/timeout/error);
/// `result` is the typed verdict a parent branches on without parsing prose
/// (PRD Stage A R-A3).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct AwaitResult {
    pub status: String,
    pub reason: Option<String>,
    pub agent_id: String,
    pub elapsed_ms: u64,
    pub workflow: Option<WorkflowSnapshot>,
    pub result: WorkflowResult,
    /// Actual run-level error cause (for example a provider/model failure),
    /// surfaced so a parent can triage without reading logs (#752). Only
    /// present when the await terminated because the child's run failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl AwaitResult {
    /// Build an `AwaitResult`, deriving the typed [`WorkflowResult`] verdict from
    /// the lifecycle status, reason, and workflow snapshot.
    pub fn new(
        status: &str,
        reason: Option<&str>,
        agent_id: String,
        elapsed_ms: u64,
        workflow: Option<WorkflowSnapshot>,
    ) -> Self {
        Self::with_error(status, reason, agent_id, elapsed_ms, workflow, None)
    }

    /// Like [`AwaitResult::new`], but carries the actual run-level error cause
    /// (#752). When `error` is present it is echoed into `result.summary` so
    /// the cause is visible in both the structured field and the prose verdict.
    pub fn with_error(
        status: &str,
        reason: Option<&str>,
        agent_id: String,
        elapsed_ms: u64,
        workflow: Option<WorkflowSnapshot>,
        error: Option<&str>,
    ) -> Self {
        let mut result = WorkflowResult::derive(status, reason, workflow.as_ref());
        if let Some(cause) = error {
            result.summary = format!("subagent run failed: {cause}");
        }
        Self {
            status: status.to_string(),
            reason: reason.map(str::to_string),
            agent_id,
            elapsed_ms,
            workflow,
            result,
            error: error.map(str::to_string),
        }
    }
}

/// Snapshot of workflow state at the moment `await` returns.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct WorkflowSnapshot {
    pub mode: String,
    pub steps_completed: u32,
    pub steps_total: u32,
}

/// Typed verdict for an awaited subagent — the structured outcome a parent can
/// branch on (PRD Stage A R-A3).
///
/// NOTE: the verdict reflects what the parent *observed* and is derived from the
/// child-reported workflow snapshot; it is NOT an integrity boundary. A
/// compromised child (which already sits inside the parent's trust boundary)
/// could influence its own reported status.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct WorkflowResult {
    pub status: VerdictStatus,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow_progress: Option<ResultProgress>,
}

/// Step progress carried in a [`WorkflowResult`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ResultProgress {
    pub done: u32,
    pub total: u32,
}

impl WorkflowResult {
    /// Derive the verdict from the await lifecycle status, reason, and workflow.
    ///
    /// - `Completed` — only when positively observed: the agent is idle AND its
    ///   workflow reached `complete`.
    /// - `Failed` — an await/agent error, or a non-clean process exit.
    /// - `Incomplete` — went idle without completing, timed out, or exited
    ///   cleanly before completion (a persistent subagent exiting is not the
    ///   success path; completion is observed at idle, never inferred from exit).
    pub fn derive(status: &str, reason: Option<&str>, workflow: Option<&WorkflowSnapshot>) -> Self {
        let workflow_progress = workflow.map(|w| ResultProgress {
            done: w.steps_completed,
            total: w.steps_total,
        });
        // Compare against the typed domain mode rather than a magic literal, so
        // a rename of WorkflowMode cannot silently regress the verdict.
        let complete = workflow.is_some_and(|w| w.mode == WorkflowMode::Complete.wire_str());
        let progress = || {
            workflow
                .map(|w| format!("{}/{} steps", w.steps_completed, w.steps_total))
                .unwrap_or_else(|| "no workflow".to_string())
        };
        let (verdict, summary): (VerdictStatus, String) = match status {
            "idle" if complete => (
                VerdictStatus::Completed,
                format!("workflow complete ({})", progress()),
            ),
            "idle" => (
                VerdictStatus::Incomplete,
                format!("went idle without completing the workflow ({})", progress()),
            ),
            "exited" => {
                let clean = reason.is_none_or(|r| r == "exit_code_0");
                if clean {
                    (
                        VerdictStatus::Incomplete,
                        "subagent exited before completion was observed".to_string(),
                    )
                } else {
                    (
                        VerdictStatus::Failed,
                        format!("subagent exited: {}", reason.unwrap_or("unknown")),
                    )
                }
            }
            "timeout" => (
                VerdictStatus::Incomplete,
                "await timed out before the subagent completed".to_string(),
            ),
            "error" => (
                VerdictStatus::Failed,
                format!("await error: {}", reason.unwrap_or("unknown")),
            ),
            other => (
                VerdictStatus::Incomplete,
                format!("subagent status: {other}"),
            ),
        };
        WorkflowResult {
            status: verdict,
            summary,
            workflow_progress,
        }
    }
}

/// Signal sent by the reaper task when a child process exits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitSignal {
    /// Process exit code (0 for success, non-zero for error).
    /// `None` if the process was killed by a signal.
    pub exit_code: Option<i32>,
    /// Signal number if the process was killed by a signal.
    pub signal: Option<i32>,
}

/// Channel for signaling process exit to a waiting `await` call.
pub type ExitSignalTx = tokio::sync::watch::Sender<Option<ExitSignal>>;
/// Receiver for process exit signals.
pub type ExitSignalRx = tokio::sync::watch::Receiver<Option<ExitSignal>>;

/// Create a new exit signal channel (initially no signal).
pub fn new_exit_signal_channel() -> (ExitSignalTx, ExitSignalRx) {
    tokio::sync::watch::channel(None)
}

/// Tracks which agent_ids have an active `await` call to prevent duplicates.
pub type ActiveAwaits = Arc<Mutex<HashSet<String>>>;

/// Create a new empty active awaits tracker.
pub fn new_active_awaits() -> ActiveAwaits {
    Arc::new(Mutex::new(HashSet::new()))
}

// ─── Subagent notifications (#523) ───────────────────────────────────────────

/// Maximum summary length for notification messages (chars).
const MAX_SUMMARY_LEN: usize = 200;

/// A notification from a child agent to the parent dispatch loop (#523).
///
/// Sent by the monitor task when a child reaches a terminal or notable state.
/// The parent dispatch loop injects these as follow-up messages to the LLM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentNotification {
    /// Child agent finished processing a prompt successfully.
    Completed { agent_id: String, summary: String },
    /// Child agent's last tool execution returned an error.
    Errored { agent_id: String, error: String },
    /// Child agent process exited (connection closed or process reaped).
    Exited { agent_id: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequencedSubagentNotification {
    pub sequence: u64,
    pub notification: SubagentNotification,
}

impl SequencedSubagentNotification {
    pub fn new(sequence: u64, notification: SubagentNotification) -> Self {
        Self {
            sequence,
            notification,
        }
    }

    pub fn dedupe_key(&self) -> (String, u64) {
        let agent_id = match &self.notification {
            SubagentNotification::Completed { agent_id, .. }
            | SubagentNotification::Errored { agent_id, .. }
            | SubagentNotification::Exited { agent_id } => agent_id.clone(),
        };
        (agent_id, self.sequence)
    }

    pub fn to_message(&self) -> String {
        self.notification.to_message()
    }
}

impl SubagentNotification {
    /// Format this notification as a human-readable message suitable for
    /// injection into the parent LLM's conversation.
    pub fn to_message(&self) -> String {
        match self {
            Self::Completed { agent_id, summary } => {
                format!(
                    "[subagent] Agent '{}' completed. Last output: {}",
                    agent_id, summary
                )
            }
            Self::Errored { agent_id, error } => {
                format!("[subagent] Agent '{}' errored: {}", agent_id, error)
            }
            Self::Exited { agent_id } => {
                format!(
                    "[subagent] Agent '{}' exited unexpectedly (process terminated)",
                    agent_id
                )
            }
        }
    }
}

/// Sender half of the notification channel.
pub type NotificationTx = tokio::sync::mpsc::Sender<SequencedSubagentNotification>;

/// Receiver half of the notification channel.
pub type NotificationRx = tokio::sync::mpsc::Receiver<SequencedSubagentNotification>;

/// Default capacity for the bounded notification channel.
pub const NOTIFICATION_CHANNEL_CAPACITY: usize = 64;

/// Create a new bounded notification channel.
pub fn new_notification_channel() -> (NotificationTx, NotificationRx) {
    tokio::sync::mpsc::channel(NOTIFICATION_CHANNEL_CAPACITY)
}

/// Extract a summary string from the `messages` array of an `agent_end` event.
///
/// Looks for the last assistant message's content text and truncates to
/// [`MAX_SUMMARY_LEN`] characters. Returns `"(no output)"` if no assistant
/// text is found.
pub fn extract_summary(messages: &serde_json::Value) -> String {
    let default = "(no output)".to_string();
    let Some(arr) = messages.as_array() else {
        return default;
    };
    // Walk backwards to find the last assistant message with content.
    for msg in arr.iter().rev() {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if role == "assistant" {
            if let Some(content) = msg.get("content").and_then(|v| v.as_str()) {
                if !content.is_empty() {
                    return truncate_summary(content);
                }
            }
        }
    }
    default
}

/// Truncate a string to [`MAX_SUMMARY_LEN`] characters, appending "..." if truncated.
/// Uses char boundary-safe slicing to avoid panics on multi-byte UTF-8.
fn truncate_summary(s: &str) -> String {
    if s.chars().count() <= MAX_SUMMARY_LEN {
        s.to_string()
    } else {
        let end = s
            .char_indices()
            .nth(MAX_SUMMARY_LEN)
            .map_or(s.len(), |(i, _)| i);
        let mut truncated = s[..end].to_string();
        truncated.push_str("...");
        truncated
    }
}

/// Validate an agent_id string for format (shared between spawn and agent_cmd).
pub fn validate_agent_id_format(agent_id: &str) -> Result<(), String> {
    let len = agent_id.len();
    if len == 0 || len > 64 {
        return Err("agent_id must be 1-64 characters".to_string());
    }
    if agent_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        Ok(())
    } else {
        Err("agent_id must use only [a-zA-Z0-9_-]".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_registry_is_empty() {
        let r = new_registry();
        assert!(r.lock().unwrap().is_empty());
    }

    #[test]
    fn verdict_completed_when_idle_and_workflow_complete() {
        let wf = WorkflowSnapshot {
            mode: "complete".into(),
            steps_completed: 7,
            steps_total: 7,
        };
        let r = WorkflowResult::derive("idle", Some("idle"), Some(&wf));
        assert_eq!(r.status, VerdictStatus::Completed);
        assert_eq!(
            r.workflow_progress,
            Some(ResultProgress { done: 7, total: 7 })
        );
    }

    #[test]
    fn verdict_incomplete_when_idle_but_workflow_active() {
        let wf = WorkflowSnapshot {
            mode: "active".into(),
            steps_completed: 3,
            steps_total: 7,
        };
        let r = WorkflowResult::derive("idle", Some("idle"), Some(&wf));
        assert_eq!(r.status, VerdictStatus::Incomplete);
    }

    #[test]
    fn verdict_incomplete_when_idle_without_workflow() {
        let r = WorkflowResult::derive("idle", Some("idle"), None);
        assert_eq!(r.status, VerdictStatus::Incomplete);
        assert!(r.workflow_progress.is_none());
    }

    #[test]
    fn verdict_failed_on_error_and_nonzero_exit() {
        assert_eq!(
            WorkflowResult::derive("error", Some("connection_failed"), None).status,
            VerdictStatus::Failed
        );
        assert_eq!(
            WorkflowResult::derive("exited", Some("exit_code_1"), None).status,
            VerdictStatus::Failed
        );
        // A clean exit is NOT completion — completion is observed at idle.
        assert_eq!(
            WorkflowResult::derive("exited", Some("exit_code_0"), None).status,
            VerdictStatus::Incomplete
        );
    }

    #[test]
    fn verdict_incomplete_on_timeout() {
        assert_eq!(
            WorkflowResult::derive("timeout", None, None).status,
            VerdictStatus::Incomplete
        );
    }

    #[test]
    fn test_validate_format_valid() {
        assert!(validate_agent_id_format("abc-123_XYZ").is_ok());
    }

    #[test]
    fn test_validate_format_empty() {
        assert!(validate_agent_id_format("").unwrap_err().contains("1-64"));
    }

    #[test]
    fn test_validate_format_too_long() {
        assert!(
            validate_agent_id_format(&"a".repeat(65))
                .unwrap_err()
                .contains("1-64")
        );
    }

    #[test]
    fn test_validate_format_special_chars() {
        assert!(
            validate_agent_id_format("a/b")
                .unwrap_err()
                .contains("[a-zA-Z0-9_-]")
        );
    }

    // --- SubagentStatus::to_wire_str ---
    #[test]
    fn test_status_wire_str_values() {
        assert_eq!(SubagentStatus::Starting.to_wire_str(), "starting");
        assert_eq!(SubagentStatus::Idle.to_wire_str(), "idle");
        assert_eq!(SubagentStatus::Running.to_wire_str(), "running");
        assert_eq!(SubagentStatus::Error.to_wire_str(), "error");
        assert_eq!(SubagentStatus::Exited.to_wire_str(), "exited");
    }

    // --- SubagentStatus ---

    #[test]
    fn test_status_display_starting() {
        assert_eq!(format!("{}", SubagentStatus::Starting), "Starting");
    }

    #[test]
    fn test_status_display_idle() {
        assert_eq!(format!("{}", SubagentStatus::Idle), "Idle");
    }

    #[test]
    fn test_status_display_running() {
        assert_eq!(format!("{}", SubagentStatus::Running), "Running");
    }

    #[test]
    fn test_status_display_error() {
        assert_eq!(format!("{}", SubagentStatus::Error), "Error");
    }

    #[test]
    fn test_status_display_exited() {
        assert_eq!(format!("{}", SubagentStatus::Exited), "Exited");
    }

    #[test]
    fn test_status_default_is_starting() {
        assert_eq!(SubagentStatus::default(), SubagentStatus::Starting);
    }

    #[test]
    fn test_all_status_variants_distinct_display() {
        let variants = [
            SubagentStatus::Starting,
            SubagentStatus::Idle,
            SubagentStatus::Running,
            SubagentStatus::Error,
            SubagentStatus::Exited,
        ];
        let displays: Vec<String> = variants.iter().map(|v| format!("{}", v)).collect();
        let unique: std::collections::HashSet<&String> = displays.iter().collect();
        assert_eq!(displays.len(), unique.len());
    }

    // --- SubagentEntry ---

    #[test]
    fn test_new_entry_has_starting_status() {
        let entry = SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 42);
        assert_eq!(entry.status, SubagentStatus::Starting);
        assert_eq!(entry.pid, 42);
        assert!(entry.last_tool.is_none());
        assert!(entry.last_error.is_none());
        assert!(entry.monitor_handle.is_none());
    }

    #[test]
    fn test_entry_socket_path() {
        let entry = SubagentEntry::new(PathBuf::from("/run/quecto.sock"), 0);
        assert_eq!(entry.socket_path, PathBuf::from("/run/quecto.sock"));
    }

    // --- SubagentNotification (#523) ---

    #[test]
    fn test_completed_message_format() {
        let n = SubagentNotification::Completed {
            agent_id: "researcher".into(),
            summary: "All tests pass".into(),
        };
        let msg = n.to_message();
        assert!(msg.starts_with("[subagent]"));
        assert!(msg.contains("researcher"));
        assert!(msg.contains("completed"));
        assert!(msg.contains("All tests pass"));
    }

    #[test]
    fn test_errored_message_format() {
        let n = SubagentNotification::Errored {
            agent_id: "linter".into(),
            error: "rate limit exceeded".into(),
        };
        let msg = n.to_message();
        assert!(msg.starts_with("[subagent]"));
        assert!(msg.contains("linter"));
        assert!(msg.contains("errored"));
        assert!(msg.contains("rate limit exceeded"));
    }

    #[test]
    fn test_exited_message_format() {
        let n = SubagentNotification::Exited {
            agent_id: "formatter".into(),
        };
        let msg = n.to_message();
        assert!(msg.starts_with("[subagent]"));
        assert!(msg.contains("formatter"));
        assert!(msg.contains("exited"));
    }

    // --- extract_summary ---

    #[test]
    fn test_extract_summary_from_assistant_message() {
        let messages = serde_json::json!([
            {"role": "user", "content": "Do something"},
            {"role": "assistant", "content": "The analysis is complete"}
        ]);
        assert_eq!(extract_summary(&messages), "The analysis is complete");
    }

    #[test]
    fn test_extract_summary_truncates_long_text() {
        let long = "x".repeat(300);
        let messages = serde_json::json!([
            {"role": "assistant", "content": long}
        ]);
        let summary = extract_summary(&messages);
        assert!(summary.len() <= 203); // 200 + "..."
        assert!(summary.ends_with("..."));
    }

    #[test]
    fn test_extract_summary_empty_messages() {
        let messages = serde_json::json!([]);
        assert_eq!(extract_summary(&messages), "(no output)");
    }

    #[test]
    fn test_extract_summary_no_assistant() {
        let messages = serde_json::json!([
            {"role": "tool", "content": "tool output"}
        ]);
        assert_eq!(extract_summary(&messages), "(no output)");
    }

    #[test]
    fn test_extract_summary_truncates_multibyte_utf8() {
        // Each emoji is 4 bytes. 201 emojis = 804 bytes but 201 chars.
        let emojis = "🦀".repeat(201);
        let messages = serde_json::json!([{"role": "assistant", "content": emojis}]);
        let summary = extract_summary(&messages);
        assert!(summary.chars().count() <= 203); // 200 chars + "..."
        assert!(summary.ends_with("..."));
        // Should not panic on multi-byte boundary
    }

    #[test]
    fn test_extract_summary_non_array() {
        let messages = serde_json::json!("not an array");
        assert_eq!(extract_summary(&messages), "(no output)");
    }

    #[test]
    fn test_extract_summary_last_assistant() {
        let messages = serde_json::json!([
            {"role": "assistant", "content": "First response"},
            {"role": "user", "content": "Another question"},
            {"role": "assistant", "content": "Second response"}
        ]);
        assert_eq!(extract_summary(&messages), "Second response");
    }

    // --- notification channel ---

    #[tokio::test]
    async fn test_notification_channel_bounded() {
        let (tx, _rx) = new_notification_channel();
        for i in 0..NOTIFICATION_CHANNEL_CAPACITY {
            let n = SubagentNotification::Completed {
                agent_id: format!("bot-{}", i),
                summary: "done".into(),
            };
            assert!(
                tx.try_send(SequencedSubagentNotification::new(i as u64 + 1, n))
                    .is_ok()
            );
        }
    }

    #[tokio::test]
    async fn test_notification_drain() {
        let (tx, mut rx) = new_notification_channel();
        for i in 0..3 {
            let _ = tx
                .send(SequencedSubagentNotification::new(
                    i as u64 + 1,
                    SubagentNotification::Exited {
                        agent_id: format!("bot-{}", i),
                    },
                ))
                .await;
        }
        drop(tx);
        let mut count = 0;
        while rx.recv().await.is_some() {
            count += 1;
        }
        assert_eq!(count, 3);
    }
}
