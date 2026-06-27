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

    /// Parse a wire-format status string back into a [`SubagentStatus`].
    /// Unknown values map to `Starting` (the conservative default). Inverse of
    /// [`to_wire_str`](Self::to_wire_str); used when merging a descendant's
    /// forwarded state into the registry (#815).
    pub fn from_wire_str(s: &str) -> Self {
        match s {
            "idle" => Self::Idle,
            "running" => Self::Running,
            "error" => Self::Error,
            "exited" => Self::Exited,
            _ => Self::Starting,
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
    /// Set true by `execute_await` when a manual `await` returns a TERMINAL
    /// result for this entry's current run. The dispatch loop check-and-consumes
    /// it before enqueuing the passive auto-note, suppressing the duplicate that
    /// a manual await would otherwise produce. Re-armed (cleared) when the entry
    /// transitions to a new run (agent_start), so a re-prompted child that
    /// completes again still notifies. See `take_completion_consumed_by_await`.
    pub completion_consumed_by_await: bool,
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
            completion_consumed_by_await: false,
        }
    }
}

/// Mark `agent_id`'s entry as having had its current-run terminal completion
/// consumed by a manual `await` (auto-await dedupe). Called by `execute_await`
/// on each terminal return path. No-op if the entry no longer exists.
pub fn mark_completion_consumed_by_await(registry: &SubagentRegistry, agent_id: &str) {
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(entry) = entries.get_mut(agent_id) {
        entry.completion_consumed_by_await = true;
    }
}

/// Check-and-consume the await-dedupe flag for `agent_id`. Returns `true` when
/// the passive completion note should be SUPPRESSED because a manual `await`
/// already reported this terminal result; in that case the flag is cleared so a
/// future re-run still notifies. Returns `false` (and leaves state untouched)
/// otherwise. Race-free against `execute_await` because the UDS dispatch loop is
/// single-threaded: the await tool call sets the flag before the loop processes
/// the queued notification.
pub fn take_completion_consumed_by_await(registry: &SubagentRegistry, agent_id: &str) -> bool {
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(entry) = entries.get_mut(agent_id) {
        if entry.completion_consumed_by_await {
            entry.completion_consumed_by_await = false;
            return true;
        }
    }
    false
}

/// Check-and-consume the await-dedupe flag for `agent_id` against an OPTIONAL
/// registry, the form both UDS dispatch paths hold (#828). Returns `true` when
/// the passive completion note should be suppressed (a manual `await` already
/// reported it); `false` when there is no registry or no pending flag. Wraps
/// [`take_completion_consumed_by_await`] so the suppress predicate lives in ONE
/// place instead of being mirrored across `uds_multi`/`uds_multi_prompt`.
pub fn consume_await_dedupe(registry: &Option<SubagentRegistry>, agent_id: &str) -> bool {
    registry
        .as_ref()
        .is_some_and(|reg| take_completion_consumed_by_await(reg, agent_id))
}

/// Shared registry of spawned subagents (agent_id → entry).
pub type SubagentRegistry = Arc<Mutex<HashMap<String, SubagentEntry>>>;

/// Create a new empty registry.
pub fn new_registry() -> SubagentRegistry {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Remove `agent_id` AND every transitive descendant (by `parent_id`) from the
/// registry, returning the ids actually removed (#831).
///
/// When an agent exits or is killed its whole sub-tree is dead: a grandchild
/// whose parent is gone can never make progress and must not linger in the root
/// registry (the lingering-panel bug). Unrelated sibling trees are untouched. A
/// missing `agent_id` is a no-op and returns an empty Vec.
pub fn cascade_remove(registry: &SubagentRegistry, agent_id: &str) -> Vec<String> {
    let mut guard = registry.lock().unwrap_or_else(|e| e.into_inner());
    if !guard.contains_key(agent_id) {
        return Vec::new();
    }
    let mut removed = Vec::new();
    let mut frontier = vec![agent_id.to_string()];
    while let Some(id) = frontier.pop() {
        if guard.remove(&id).is_none() {
            continue;
        }
        // Any entry whose parent is the just-removed id is now orphaned.
        for (child_id, entry) in guard.iter() {
            if entry.parent_id.as_deref() == Some(id.as_str()) {
                frontier.push(child_id.clone());
            }
        }
        removed.push(id);
    }
    removed
}

/// Maximum wall-clock time to wait for a forwarded sub-agent UDS response on the
/// `agent_cmd` path (a tool call that may legitimately wait on a long operation).
const SUBAGENT_RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Short, interactive-scale timeout for forwards driven by the TUI inspector's
/// 1s poll loop (#795). A `get_messages_tail` query answers from history almost
/// instantly; capping at a few seconds keeps a slow/hung sub-agent from
/// head-of-line-blocking the parent's shared dispatch loop (review: DoS / perf).
pub const INSPECTOR_RESPONSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Per-line cap on a sub-agent's UDS reply (#795 security review). Mirrors the
/// inbound client cap (`uds::MAX_LINE_BYTES`) so a misbehaving/compromised
/// sub-agent cannot return an unbounded line and exhaust the parent's memory.
const SUBAGENT_RESPONSE_MAX_LINE_BYTES: usize = 1024 * 1024;

/// Look up the UDS socket path for a spawned sub-agent by id.
///
/// Single source of truth for both `agent_cmd` forwarding and the TUI's
/// agent-targeted `get_messages_tail` so the lookup rule never diverges (#795).
pub fn lookup_subagent_socket(
    registry: &SubagentRegistry,
    agent_id: &str,
) -> Result<PathBuf, String> {
    let entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    entries
        .get(agent_id)
        .map(|e| e.socket_path.clone())
        .ok_or_else(|| format!("subagent '{}' not found in registry", agent_id))
}

/// Send a JSON-lines command to a sub-agent's UDS socket and read the first
/// `response` line back.
///
/// Each call opens a new connection, writes `command` + newline, and reads
/// lines until a `{"type":"response",...}` event arrives (skipping tokens,
/// agent_start, etc. that the broadcast delivers to all clients). Shared by
/// `agent_cmd` and the TUI agent-targeted tail forwarder so the framing rule
/// lives in one place (#795).
///
/// Uses the long [`SUBAGENT_RESPONSE_TIMEOUT`]; interactive callers that must
/// not block the shared dispatch loop should use
/// [`send_subagent_uds_command_with_timeout`] with [`INSPECTOR_RESPONSE_TIMEOUT`].
pub async fn send_subagent_uds_command(
    socket_path: &std::path::Path,
    command: &str,
) -> Result<String, crate::domain::error::DomainError> {
    send_subagent_uds_command_with_timeout(socket_path, command, SUBAGENT_RESPONSE_TIMEOUT).await
}

/// Like [`send_subagent_uds_command`] but with an explicit response timeout, so
/// interactive callers (the TUI inspector poll, #795) can cap head-of-line
/// blocking on the parent's shared command loop.
pub async fn send_subagent_uds_command_with_timeout(
    socket_path: &std::path::Path,
    command: &str,
    response_timeout: std::time::Duration,
) -> Result<String, crate::domain::error::DomainError> {
    use crate::domain::error::DomainError;
    use tokio::io::{AsyncWriteExt, BufReader};

    let stream = tokio::net::UnixStream::connect(socket_path)
        .await
        .map_err(|e| {
            DomainError::Tool(format!(
                "connect to subagent at {} failed: {e}",
                socket_path.display()
            ))
        })?;

    let (reader, mut writer) = tokio::io::split(stream);

    writer
        .write_all(command.as_bytes())
        .await
        .map_err(|e| DomainError::Tool(format!("write to subagent failed: {e}")))?;
    writer
        .write_all(b"\n")
        .await
        .map_err(|e| DomainError::Tool(format!("write to subagent failed: {e}")))?;
    // Do NOT shutdown or drop the write half (#557): in multi-client mode the
    // server's reader loop exits on EOF and aborts the broadcast writer, so the
    // response would never arrive. Keep the write half alive until we're done.
    let _keep_alive = writer;

    let mut reader = BufReader::new(reader);
    let deadline = tokio::time::Instant::now() + response_timeout;
    let timeout_msg = || {
        format!(
            "subagent response timed out ({}s)",
            response_timeout.as_secs()
        )
    };
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(DomainError::Tool(timeout_msg()));
        }
        let line = tokio::time::timeout(
            remaining,
            read_line_capped(&mut reader, SUBAGENT_RESPONSE_MAX_LINE_BYTES),
        )
        .await
        .map_err(|_| DomainError::Tool(timeout_msg()))??;
        match line {
            Some(l) => {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&l) {
                    if json.get("type").and_then(|v| v.as_str()) == Some("response") {
                        return Ok(l);
                    }
                }
                // Not a response event — skip.
            }
            None => {
                return Err(DomainError::Tool(
                    "subagent closed connection without sending a response".into(),
                ));
            }
        }
    }
}

/// Read a single `\n`-terminated line, rejecting (rather than buffering) any line
/// that exceeds `max_bytes` (#795 security review). Unlike `AsyncBufReadExt::lines`,
/// this caps each line so a sub-agent cannot OOM the parent with one giant line,
/// while still allowing an unbounded number of normal-sized lines to be skipped
/// before the `response` event arrives.
async fn read_line_capped<R>(
    reader: &mut R,
    max_bytes: usize,
) -> Result<Option<String>, crate::domain::error::DomainError>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    use crate::domain::error::DomainError;
    use tokio::io::AsyncBufReadExt;

    let mut buf: Vec<u8> = Vec::new();
    loop {
        let available = reader
            .fill_buf()
            .await
            .map_err(|e| DomainError::Tool(format!("read from subagent failed: {e}")))?;
        if available.is_empty() {
            // EOF: surface any trailing partial line, else signal closed stream.
            return Ok((!buf.is_empty()).then(|| String::from_utf8_lossy(&buf).into_owned()));
        }
        if let Some(pos) = available.iter().position(|&b| b == b'\n') {
            buf.extend_from_slice(&available[..pos]);
            reader.consume(pos + 1);
            if buf.len() > max_bytes {
                return Err(DomainError::Tool(
                    "subagent response line exceeded size limit".into(),
                ));
            }
            return Ok(Some(String::from_utf8_lossy(&buf).into_owned()));
        }
        let consumed = available.len();
        buf.extend_from_slice(available);
        reader.consume(consumed);
        if buf.len() > max_bytes {
            return Err(DomainError::Tool(
                "subagent response line exceeded size limit".into(),
            ));
        }
    }
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
        // Keep this a single concise line — it surfaces as a passive one-line note
        // in the TUI and as a system note in the parent's context. The child's full
        // output is NOT repeated here; inspect it with `agent_cmd get_messages_tail`.
        match self {
            Self::Completed { agent_id, .. } => {
                format!("Agent '{agent_id}' completed and is ready for inspection")
            }
            Self::Errored { agent_id, error } => {
                format!("Agent '{agent_id}' failed: {error}")
            }
            Self::Exited { agent_id } => {
                format!("Agent '{agent_id}' exited unexpectedly")
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
