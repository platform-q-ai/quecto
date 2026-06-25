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
    /// (#752). The cause is redacted (see [`redact_secrets`]) before it crosses
    /// the trust boundary into the parent context, then threaded into
    /// [`WorkflowResult::derive`] so the verdict and summary stay derived in one
    /// place; the same redacted value populates the structured `error` field.
    pub fn with_error(
        status: &str,
        reason: Option<&str>,
        agent_id: String,
        elapsed_ms: u64,
        workflow: Option<WorkflowSnapshot>,
        error: Option<&str>,
    ) -> Self {
        // Redact once: provider/HTTP error strings can embed secrets (bearer
        // tokens, api keys, auth headers) and these responses reach the parent
        // model context and logs (#752, security review).
        let redacted = error.map(redact_secrets);
        let result = WorkflowResult::derive(status, reason, workflow.as_ref(), redacted.as_deref());
        Self {
            status: status.to_string(),
            reason: reason.map(str::to_string),
            agent_id,
            elapsed_ms,
            workflow,
            result,
            error: redacted,
        }
    }
}

/// Strip known secret patterns and bound the length of an error cause before it
/// is surfaced to the parent agent (#752). This is defense-in-depth: provider
/// error strings are not guaranteed to be sanitized upstream and can echo
/// bearer tokens, API keys, or auth query params.
///
/// Thin wrapper over the shared [`crate::domain::redaction`] redactor (single
/// source of truth, also used by the audit log, #790) that adds the length
/// bound specific to parent-context surfacing.
fn redact_secrets(cause: &str) -> String {
    const MAX_LEN: usize = 2000;
    crate::domain::redaction::redact_and_bound(cause, MAX_LEN)
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
    pub fn derive(
        status: &str,
        reason: Option<&str>,
        workflow: Option<&WorkflowSnapshot>,
        error: Option<&str>,
    ) -> Self {
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
            // Keep the reason context the verdict produced and append the
            // concrete run-level cause when one was surfaced (#752), so the
            // summary stays derived here rather than post-mutated by callers.
            "error" => {
                let base = format!("await error: {}", reason.unwrap_or("unknown"));
                let summary = match error {
                    Some(cause) => format!("{base} — {cause}"),
                    None => base,
                };
                (VerdictStatus::Failed, summary)
            }
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
#[path = "subagent_registry_tests.rs"]
mod tests;
