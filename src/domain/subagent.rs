// Subagent domain types: configuration and validation.

use std::path::PathBuf;

use super::error::DomainError;

/// Configuration for spawning a subagent.
#[derive(Debug, Clone)]
pub struct SubagentConfig {
    /// The task to execute (optional — agent starts idle if omitted).
    pub task: Option<String>,
    /// Optional target agent ID.
    pub agent_id: Option<String>,
    /// Whether the subagent should restrict to workspace.
    pub restrict_to_workspace: bool,
    /// Optional system prompt for the subagent.
    pub system: Option<String>,
    /// Optional config file path to forward as `--config <path>`.
    pub config_path: Option<PathBuf>,
    /// Whether to start the child with `--workflow`.
    pub workflow: bool,
    /// Whether to start the child with `--workflow-guards`.
    pub workflow_guards: bool,
    /// Optional by-value workflow assignment (raw JSON of a [`WorkflowSpec`]).
    /// When set, the child is launched with `--workflow-spec <path>` and runs
    /// exactly that template in Active mode (binding). [`WorkflowSpec`] lives in
    /// `crate::domain::workflow`.
    pub workflow_spec_json: Option<String>,
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

    #[test]
    fn test_subagent_config_new_fields_default() {
        let cfg = SubagentConfig {
            task: None,
            agent_id: None,
            restrict_to_workspace: true,
            system: None,
            config_path: None,
            workflow: false,
            workflow_guards: false,
            workflow_spec_json: None,
        };
        assert!(cfg.config_path.is_none());
        assert!(!cfg.workflow);
        assert!(!cfg.workflow_guards);
    }

    #[test]
    fn test_subagent_config_with_config_path() {
        let cfg = SubagentConfig {
            task: None,
            agent_id: None,
            restrict_to_workspace: true,
            system: None,
            config_path: Some(PathBuf::from("/custom/config.json")),
            workflow: false,
            workflow_guards: false,
            workflow_spec_json: None,
        };
        assert_eq!(cfg.config_path, Some(PathBuf::from("/custom/config.json")));
    }

    #[test]
    fn test_subagent_config_with_workflow() {
        let cfg = SubagentConfig {
            task: None,
            agent_id: None,
            restrict_to_workspace: true,
            system: None,
            config_path: None,
            workflow: true,
            workflow_guards: true,
            workflow_spec_json: None,
        };
        assert!(cfg.workflow);
        assert!(cfg.workflow_guards);
    }
}
