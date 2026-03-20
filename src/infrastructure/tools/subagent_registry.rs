// Shared subagent registry types for spawn + agent_cmd tools (#421).
// Extended with live status tracking for persistent monitor (#522).

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

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
    /// Description of the last error (from tool_execution_end with is_error).
    pub last_error: Option<String>,
    /// When this entry was last updated by the monitor.
    pub updated_at: Instant,
    /// Abort handle for the monitor task (if running).
    pub monitor_handle: Option<Arc<tokio::task::JoinHandle<()>>>,
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
            updated_at: Instant::now(),
            monitor_handle: None,
        }
    }
}

/// Shared registry of spawned subagents (agent_id → entry).
pub type SubagentRegistry = Arc<Mutex<HashMap<String, SubagentEntry>>>;

/// Create a new empty registry.
pub fn new_registry() -> SubagentRegistry {
    Arc::new(Mutex::new(HashMap::new()))
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
pub type NotificationTx = tokio::sync::mpsc::Sender<SubagentNotification>;

/// Receiver half of the notification channel.
pub type NotificationRx = tokio::sync::mpsc::Receiver<SubagentNotification>;

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

/// Truncate a string to [`MAX_SUMMARY_LEN`] characters, appending "…" if truncated.
fn truncate_summary(s: &str) -> String {
    if s.len() <= MAX_SUMMARY_LEN {
        s.to_string()
    } else {
        let mut truncated = s[..MAX_SUMMARY_LEN].to_string();
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
            assert!(tx.try_send(n).is_ok());
        }
    }

    #[tokio::test]
    async fn test_notification_drain() {
        let (tx, mut rx) = new_notification_channel();
        for i in 0..3 {
            let _ = tx
                .send(SubagentNotification::Exited {
                    agent_id: format!("bot-{}", i),
                })
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
