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
}
