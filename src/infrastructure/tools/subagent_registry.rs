// Shared subagent registry types for spawn + agent_cmd tools (#421).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Entry for a spawned subagent in the shared registry.
#[derive(Debug, Clone)]
pub struct SubagentEntry {
    /// Path to the child's UDS socket.
    pub socket_path: PathBuf,
    /// Child process PID (0 in stub mode).
    pub pid: u32,
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
}
