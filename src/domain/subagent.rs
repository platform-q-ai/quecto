// Subagent domain types: configuration and validation.

use super::error::DomainError;

/// Configuration for spawning a subagent.
#[derive(Debug, Clone)]
pub struct SubagentConfig {
    /// The task to execute.
    pub task: String,
    /// Optional target agent ID.
    pub agent_id: Option<String>,
    /// Whether the subagent should restrict to workspace.
    pub restrict_to_workspace: bool,
    /// Optional system prompt for the subagent.
    pub system: Option<String>,
}

/// Validate an agent_id against an allowlist.
/// Returns Ok if the agent_id is in the allowlist, or Err if not.
pub fn validate_agent_id(agent_id: &str, allowlist: &[String]) -> Result<(), DomainError> {
    if allowlist.iter().any(|id| id == agent_id) {
        Ok(())
    } else {
        Err(DomainError::Security(format!(
            "agent_id '{}' is not allowed",
            agent_id
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_agent_id_allowed() {
        let allowlist = vec!["news-bot".to_string(), "weather-bot".to_string()];
        assert!(validate_agent_id("news-bot", &allowlist).is_ok());
    }

    #[test]
    fn test_validate_agent_id_rejected() {
        let allowlist = vec!["news-bot".to_string()];
        let result = validate_agent_id("evil-bot", &allowlist);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not allowed"));
    }
}
