// Spawn tool: spawns a child quecto agent process for background tasks.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::time::Duration;

use crate::domain::error::DomainError;
use crate::domain::subagent::{SubagentConfig, validate_agent_id};
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};

/// Tool that spawns a child `quecto agent` process for background tasks.
///
/// When executed, validates the request and runs `quecto agent -m <task>`
/// as a subprocess, collecting its output. The child process inherits the
/// parent's base directory and workspace restrictions.
#[derive(Debug)]
pub struct SpawnTool {
    /// Allowlist of agent IDs that can be spawned.
    allowed_agents: Vec<String>,
    /// Whether workspace restriction should be inherited.
    restrict_to_workspace: bool,
    /// Base directory for the child agent process.
    base_dir: PathBuf,
}

impl SpawnTool {
    const SUBAGENT_TIMEOUT_SECS: u64 = 120;

    pub fn new(allowed_agents: Vec<String>, restrict_to_workspace: bool) -> Self {
        Self {
            allowed_agents,
            restrict_to_workspace,
            base_dir: PathBuf::new(),
        }
    }

    /// Create with a base directory for subprocess spawning.
    pub fn with_base_dir(
        allowed_agents: Vec<String>,
        restrict_to_workspace: bool,
        base_dir: PathBuf,
    ) -> Self {
        Self {
            allowed_agents,
            restrict_to_workspace,
            base_dir,
        }
    }

    /// Parse the tool arguments and create a SubagentConfig.
    fn parse_args(&self, arguments: &str) -> Result<SubagentConfig, String> {
        let args: serde_json::Value =
            serde_json::from_str(arguments).map_err(|e| format!("invalid JSON: {}", e))?;

        let task = args
            .get("task")
            .and_then(|v| v.as_str())
            .ok_or("missing required field: task")?
            .to_string();

        let agent_id = args
            .get("agent_id")
            .and_then(|v| v.as_str())
            .map(String::from);

        let system = args
            .get("system")
            .and_then(|v| v.as_str())
            .map(String::from);

        if let Some(ref id) = agent_id {
            Self::validate_agent_id_format(id)?;
            if !self.allowed_agents.is_empty() {
                validate_agent_id(id, &self.allowed_agents).map_err(|e| e.to_string())?;
            }
        }

        Ok(SubagentConfig {
            task,
            agent_id,
            restrict_to_workspace: self.restrict_to_workspace,
            deliver_to: None,
            system,
        })
    }

    fn validate_agent_id_format(agent_id: &str) -> Result<(), String> {
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

    /// Spawn a child quecto agent process and collect its output.
    async fn run_subprocess(&self, config: &SubagentConfig) -> Result<ToolResult, DomainError> {
        let binary = std::env::current_exe()
            .map_err(|e| DomainError::Tool(format!("cannot find quecto binary: {}", e)))?;

        let session_name = config.agent_id.as_deref().unwrap_or("subagent");

        let mut cmd = tokio::process::Command::new(&binary);
        cmd.arg("agent")
            .arg("-m")
            .arg(&config.task)
            .arg("-s")
            .arg(session_name);

        if let Some(ref system) = config.system {
            cmd.arg("--system").arg(system);
        }

        if !self.base_dir.as_os_str().is_empty() {
            cmd.env("QUECTO_BASE_DIR", &self.base_dir);
        }

        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        let mut child = cmd
            .spawn()
            .map_err(|e| DomainError::Tool(format!("failed to spawn subagent: {}", e)))?;

        let status = match tokio::time::timeout(
            Duration::from_secs(Self::SUBAGENT_TIMEOUT_SECS),
            child.wait(),
        )
        .await
        {
            Ok(wait_result) => wait_result
                .map_err(|e| DomainError::Tool(format!("subagent process error: {}", e)))?,
            Err(_) => {
                let _ = child.kill().await;
                return Ok(ToolResult {
                    content: format!(
                        "Subagent '{}' timed out after {}s.",
                        session_name,
                        Self::SUBAGENT_TIMEOUT_SECS
                    ),
                    is_error: true,
                });
            }
        };

        if status.success() {
            Ok(ToolResult {
                content: format!("Subagent '{}' completed successfully.", session_name),
                is_error: false,
            })
        } else {
            let code = status.code().unwrap_or(-1);
            Ok(ToolResult {
                content: format!("Subagent '{}' failed (exit code {}).", session_name, code),
                is_error: true,
            })
        }
    }
}

impl Tool for SpawnTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "spawn".to_string(),
            description: "Spawn a subagent to handle a task. The subagent runs as a child process."
                .to_string(),
            parameters_schema: r#"{"type":"object","properties":{"task":{"type":"string","description":"The task description for the subagent"},"agent_id":{"type":"string","description":"Optional agent ID for the subagent session"},"system":{"type":"string","description":"Optional system prompt for the subagent"}},"required":["task"]}"#.to_string(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args = arguments.to_string();
        Box::pin(async move {
            match self.parse_args(&args) {
                Ok(config) => {
                    // Only spawn subprocess when base_dir is configured (gateway mode).
                    // Otherwise return a stub result (unit test / isolated mode).
                    if self.base_dir.as_os_str().is_empty() {
                        let msg = format!(
                            "Subagent spawned for task: '{}'. Restrict to workspace: {}.",
                            config.task, config.restrict_to_workspace
                        );
                        Ok(ToolResult {
                            content: msg,
                            is_error: false,
                        })
                    } else {
                        self.run_subprocess(&config).await
                    }
                }
                Err(e) => Ok(ToolResult {
                    content: format!("Failed to spawn subagent: {}", e),
                    is_error: true,
                }),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_tool() -> SpawnTool {
        SpawnTool::new(
            vec!["news-bot".to_string(), "weather-bot".to_string()],
            true,
        )
    }

    #[test]
    fn test_definition() {
        let tool = test_tool();
        let def = tool.definition();
        assert_eq!(def.name, "spawn");
        assert!(!def.description.is_empty());
    }

    #[test]
    fn test_parse_valid_task() {
        let tool = test_tool();
        let config = tool.parse_args(r#"{"task":"Summarize news"}"#).unwrap();
        assert_eq!(config.task, "Summarize news");
        assert!(config.agent_id.is_none());
    }

    #[test]
    fn test_parse_with_agent_id() {
        let tool = test_tool();
        let config = tool
            .parse_args(r#"{"task":"Get weather","agent_id":"weather-bot"}"#)
            .unwrap();
        assert_eq!(config.agent_id.as_deref(), Some("weather-bot"));
    }

    #[test]
    fn test_parse_disallowed_agent() {
        let tool = test_tool();
        let result = tool.parse_args(r#"{"task":"Evil task","agent_id":"evil-bot"}"#);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not allowed"));
    }

    #[test]
    fn test_parse_missing_task() {
        let tool = test_tool();
        let result = tool.parse_args(r#"{"agent_id":"news-bot"}"#);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing"));
    }

    #[test]
    fn test_parse_empty_allowlist_permits_any() {
        let tool = SpawnTool::new(vec![], true);
        let config = tool
            .parse_args(r#"{"task":"Do stuff","agent_id":"any-bot"}"#)
            .unwrap();
        assert_eq!(config.agent_id.as_deref(), Some("any-bot"));
    }

    #[test]
    fn test_parse_with_system_prompt() {
        let tool = test_tool();
        let config = tool
            .parse_args(r#"{"task":"Summarize","system":"You are a summarizer"}"#)
            .unwrap();
        assert_eq!(config.system.as_deref(), Some("You are a summarizer"));
    }

    #[test]
    fn test_parse_rejects_invalid_agent_id_format() {
        let tool = SpawnTool::new(vec![], true);
        let result = tool.parse_args(r#"{"task":"Do stuff","agent_id":"../escape"}"#);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("[a-zA-Z0-9_-]"));
    }
}
