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
    /// Optional by-value workflow assignment. When set, the child is launched
    /// with `--workflow-spec <path>` and runs exactly that template in Active
    /// mode (binding).
    pub workflow_spec: Option<crate::domain::workflow::WorkflowSpec>,
    /// Optional model override, forwarded to the child as `--model <value>`.
    /// Resolved to the canonical `provider/model` form. When `None`, the child
    /// resolves its model from the inherited `--config` or the built-in default.
    pub model: Option<String>,
    /// Tool names to remove from the child's registry before its session starts
    /// (forwarded as `--disable-tool <name>` per entry). Empty means no tools are
    /// disabled. Used to launch read-only children (e.g. reviewers) with `write`
    /// and `edit` removed so the model never sees them (#957).
    pub disable_tools: Vec<String>,
}

/// A validated model argument, in either of the two forms accepted by
/// `set_model` and `spawn`: a single `provider/model` string, or a separate
/// `provider` + `model_id` pair.
///
/// This is the single source of truth for model-argument validation so the
/// `spawn` tool and `agent_cmd set_model` cannot diverge (#881).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelArg {
    /// A full `provider/model` string (e.g. `openai/gpt-5.5`).
    Full(String),
    /// A separate provider + model id pair (e.g. `openai` + `gpt-5.5`).
    Pair { provider: String, model_id: String },
}

impl ModelArg {
    /// Collapse to the canonical single-string `provider/model` form used by
    /// the `--model` CLI flag and the config default.
    pub fn to_model_string(&self) -> String {
        match self {
            ModelArg::Full(m) => m.clone(),
            ModelArg::Pair { provider, model_id } => format!("{provider}/{model_id}"),
        }
    }
}

/// Validate the `model` / `provider` / `model_id` argument trio shared by
/// `set_model` and `spawn`. Empty strings are treated as absent. Exactly one of
/// (`model`) or (`provider` + `model_id`) must be supplied.
///
/// Returns `Ok(None)` when none are supplied (caller falls back to its default
/// behaviour) and `Err` with a clear message when the combination is invalid.
pub fn parse_model_arg(
    model: Option<&str>,
    provider: Option<&str>,
    model_id: Option<&str>,
) -> Result<Option<ModelArg>, String> {
    let model = model.map(str::trim).filter(|s| !s.is_empty());
    let provider = provider.map(str::trim).filter(|s| !s.is_empty());
    let model_id = model_id.map(str::trim).filter(|s| !s.is_empty());

    match (model, provider, model_id) {
        (Some(m), _, _) => Ok(Some(ModelArg::Full(m.to_string()))),
        (None, Some(p), Some(mid)) => Ok(Some(ModelArg::Pair {
            provider: p.to_string(),
            model_id: mid.to_string(),
        })),
        (None, Some(_), None) => Err("provider requires model_id".to_string()),
        (None, None, Some(_)) => Err("model_id requires provider".to_string()),
        (None, None, None) => Ok(None),
    }
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
            workflow_spec: None,
            model: None,
            disable_tools: Vec::new(),
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
            workflow_spec: None,
            model: None,
            disable_tools: Vec::new(),
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
            workflow_spec: None,
            model: None,
            disable_tools: Vec::new(),
        };
        assert!(cfg.workflow);
        assert!(cfg.workflow_guards);
    }

    // --- parse_model_arg (#881) ---

    #[test]
    fn test_parse_model_arg_full_string() {
        let arg = parse_model_arg(Some("openai/gpt-5.5"), None, None)
            .unwrap()
            .unwrap();
        assert_eq!(arg, ModelArg::Full("openai/gpt-5.5".to_string()));
        assert_eq!(arg.to_model_string(), "openai/gpt-5.5");
    }

    #[test]
    fn test_parse_model_arg_provider_model_id_pair() {
        let arg = parse_model_arg(None, Some("openai"), Some("gpt-5.5"))
            .unwrap()
            .unwrap();
        assert_eq!(arg.to_model_string(), "openai/gpt-5.5");
    }

    #[test]
    fn test_parse_model_arg_none_is_ok_none() {
        assert_eq!(parse_model_arg(None, None, None).unwrap(), None);
        // Empty strings are treated as absent.
        assert_eq!(parse_model_arg(Some(""), Some(""), Some("")).unwrap(), None);
    }

    #[test]
    fn test_parse_model_arg_provider_without_model_id_errors() {
        let err = parse_model_arg(None, Some("openai"), None).unwrap_err();
        assert!(err.contains("model_id"), "got: {err}");
    }

    #[test]
    fn test_parse_model_arg_model_id_without_provider_errors() {
        let err = parse_model_arg(None, None, Some("gpt-5.5")).unwrap_err();
        assert!(err.contains("provider"), "got: {err}");
    }

    #[test]
    fn test_parse_model_arg_full_takes_precedence_over_pair() {
        let arg = parse_model_arg(Some("a/b"), Some("openai"), Some("gpt-5.5"))
            .unwrap()
            .unwrap();
        assert_eq!(arg, ModelArg::Full("a/b".to_string()));
    }
}
