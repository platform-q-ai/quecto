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
    const SUBAGENT_TIMEOUT_SECS: u64 = 86_400; // 24 hours

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

        // Propagate --no-sandbox so child agents inherit the same workspace
        // restriction posture as the parent. Without this, the child would
        // silently re-enable the sandbox by re-reading config defaults.
        if !self.restrict_to_workspace {
            cmd.arg("--no-sandbox");
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
                // Reap deterministically — avoids zombie if the runtime is
                // shutting down (e.g. parent hit --max-time) before Tokio's
                // background reaper can run.
                let _ = child.wait().await;
                return Ok(ToolResult {
                    content: format!(
                        "Subagent '{}' timed out after {}s.",
                        session_name,
                        Self::SUBAGENT_TIMEOUT_SECS
                    ),
                    is_error: true,
                    image_blocks: vec![],
                });
            }
        };

        if status.success() {
            Ok(ToolResult {
                content: format!("Subagent '{}' completed successfully.", session_name),
                is_error: false,
                image_blocks: vec![],
            })
        } else {
            let code = status.code().unwrap_or(-1);
            Ok(ToolResult {
                content: format!("Subagent '{}' failed (exit code {}).", session_name, code),
                is_error: true,
                image_blocks: vec![],
            })
        }
    }
}

impl Tool for SpawnTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "spawn".into(),
            description: "Spawn a subagent to handle a task. The subagent runs as a child process."
                .into(),
            parameters_schema: r#"{"type":"object","properties":{"task":{"type":"string","description":"The task description for the subagent"},"agent_id":{"type":"string","description":"Optional agent ID for the subagent session"},"system":{"type":"string","description":"Optional system prompt for the subagent"}},"required":["task"]}"#.into(),
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
                            image_blocks: vec![],
                        })
                    } else {
                        self.run_subprocess(&config).await
                    }
                }
                Err(e) => Ok(ToolResult {
                    content: format!("Failed to spawn subagent: {}", e),
                    is_error: true,
                    image_blocks: vec![],
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

    // --- with_base_dir constructor ---

    #[test]
    fn test_with_base_dir_sets_fields() {
        let base = PathBuf::from("/tmp/quecto-test");
        let tool = SpawnTool::with_base_dir(vec!["bot-a".to_string()], false, base.clone());
        assert_eq!(tool.base_dir, base);
        assert_eq!(tool.allowed_agents, vec!["bot-a".to_string()]);
        assert!(!tool.restrict_to_workspace);
    }

    #[test]
    fn test_new_sets_empty_base_dir() {
        let tool = SpawnTool::new(vec![], false);
        assert!(tool.base_dir.as_os_str().is_empty());
    }

    // --- validate_agent_id_format ---

    #[test]
    fn test_validate_agent_id_format_empty_string() {
        let result = SpawnTool::validate_agent_id_format("");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("1-64 characters"));
    }

    #[test]
    fn test_validate_agent_id_format_max_length_64() {
        let id = "a".repeat(64);
        let result = SpawnTool::validate_agent_id_format(&id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_agent_id_format_too_long_65() {
        let id = "a".repeat(65);
        let result = SpawnTool::validate_agent_id_format(&id);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("1-64 characters"));
    }

    #[test]
    fn test_validate_agent_id_format_all_valid_chars() {
        assert!(SpawnTool::validate_agent_id_format("abcXYZ019_-").is_ok());
    }

    #[test]
    fn test_validate_agent_id_format_single_char() {
        assert!(SpawnTool::validate_agent_id_format("a").is_ok());
        assert!(SpawnTool::validate_agent_id_format("Z").is_ok());
        assert!(SpawnTool::validate_agent_id_format("0").is_ok());
        assert!(SpawnTool::validate_agent_id_format("_").is_ok());
        assert!(SpawnTool::validate_agent_id_format("-").is_ok());
    }

    #[test]
    fn test_validate_agent_id_format_invalid_dot() {
        let result = SpawnTool::validate_agent_id_format("hello.world");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("[a-zA-Z0-9_-]"));
    }

    #[test]
    fn test_validate_agent_id_format_invalid_space() {
        let result = SpawnTool::validate_agent_id_format("hello world");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("[a-zA-Z0-9_-]"));
    }

    #[test]
    fn test_validate_agent_id_format_invalid_slash() {
        let result = SpawnTool::validate_agent_id_format("a/b");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_agent_id_format_invalid_unicode() {
        let result = SpawnTool::validate_agent_id_format("böt");
        assert!(result.is_err());
    }

    // --- execute() stub mode (empty base_dir) ---

    #[tokio::test]
    async fn test_execute_stub_mode_success() {
        let tool = SpawnTool::new(vec![], true);
        let result = tool
            .execute(r#"{"task":"Do something useful"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Do something useful"));
        assert!(result.content.contains("Restrict to workspace: true"));
    }

    #[tokio::test]
    async fn test_execute_stub_mode_restrict_false() {
        let tool = SpawnTool::new(vec![], false);
        let result = tool.execute(r#"{"task":"background job"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Restrict to workspace: false"));
    }

    #[tokio::test]
    async fn test_execute_stub_mode_with_agent_id() {
        let tool = SpawnTool::new(vec!["my-bot".to_string()], true);
        let result = tool
            .execute(r#"{"task":"fetch data","agent_id":"my-bot"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("fetch data"));
    }

    // --- execute() with invalid input ---

    #[tokio::test]
    async fn test_execute_invalid_json() {
        let tool = SpawnTool::new(vec![], true);
        let result = tool.execute("not valid json").await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Failed to spawn subagent"));
        assert!(result.content.contains("invalid JSON"));
    }

    #[tokio::test]
    async fn test_execute_missing_task_field() {
        let tool = SpawnTool::new(vec![], true);
        let result = tool.execute(r#"{"agent_id":"bot"}"#).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("missing"));
    }

    #[tokio::test]
    async fn test_execute_disallowed_agent_returns_error() {
        let tool = SpawnTool::new(vec!["allowed-bot".to_string()], true);
        let result = tool
            .execute(r#"{"task":"evil","agent_id":"not-allowed"}"#)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not allowed"));
    }

    #[tokio::test]
    async fn test_execute_invalid_agent_id_format_returns_error() {
        let tool = SpawnTool::new(vec![], true);
        let result = tool
            .execute(r#"{"task":"test","agent_id":"bad id!"}"#)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("[a-zA-Z0-9_-]"));
    }

    // --- parse_args edge cases ---

    #[test]
    fn test_parse_args_invalid_json_garbage() {
        let tool = test_tool();
        let result = tool.parse_args("{garbage}}}");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid JSON"));
    }

    #[test]
    fn test_parse_args_task_not_string() {
        let tool = test_tool();
        let result = tool.parse_args(r#"{"task":42}"#);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing"));
    }

    #[test]
    fn test_parse_args_task_null() {
        let tool = test_tool();
        let result = tool.parse_args(r#"{"task":null}"#);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing"));
    }

    #[test]
    fn test_parse_args_empty_object() {
        let tool = test_tool();
        let result = tool.parse_args(r#"{}"#);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing"));
    }

    #[test]
    fn test_parse_args_system_not_string_ignored() {
        let tool = test_tool();
        // system is a number — as_str() returns None, so system should be None
        let config = tool.parse_args(r#"{"task":"work","system":123}"#).unwrap();
        assert!(config.system.is_none());
    }

    #[test]
    fn test_parse_args_agent_id_not_string_ignored() {
        let tool = test_tool();
        // agent_id is a number — as_str() returns None, so agent_id should be None
        let config = tool
            .parse_args(r#"{"task":"work","agent_id":999}"#)
            .unwrap();
        assert!(config.agent_id.is_none());
    }

    #[test]
    fn test_parse_args_deliver_to_always_none() {
        let tool = test_tool();
        let config = tool.parse_args(r#"{"task":"work"}"#).unwrap();
        assert!(config.deliver_to.is_none());
    }

    #[test]
    fn test_parse_args_restrict_to_workspace_inherited() {
        let tool_true = SpawnTool::new(vec![], true);
        let tool_false = SpawnTool::new(vec![], false);
        let cfg_t = tool_true.parse_args(r#"{"task":"a"}"#).unwrap();
        let cfg_f = tool_false.parse_args(r#"{"task":"a"}"#).unwrap();
        assert!(cfg_t.restrict_to_workspace);
        assert!(!cfg_f.restrict_to_workspace);
    }

    // --- Debug trait ---

    #[test]
    fn test_debug_trait() {
        let tool = SpawnTool::new(vec!["bot".to_string()], true);
        let debug_str = format!("{:?}", tool);
        assert!(debug_str.contains("SpawnTool"));
        assert!(debug_str.contains("bot"));
        assert!(debug_str.contains("restrict_to_workspace: true"));
    }

    #[test]
    fn test_debug_with_base_dir() {
        let tool = SpawnTool::with_base_dir(vec![], false, PathBuf::from("/some/path"));
        let debug_str = format!("{:?}", tool);
        assert!(debug_str.contains("/some/path"));
    }

    // --- Timeout constant ---

    #[test]
    fn test_subagent_timeout_is_at_least_one_hour() {
        // Subagent tasks are long-running; the timeout must be ≥ 1 h (3600s).
        // The intended default is 24 h (86400s) but this guards against the
        // timeout being accidentally dropped back to a short value.
        const { assert!(SpawnTool::SUBAGENT_TIMEOUT_SECS >= 3_600) };
    }

    #[test]
    fn test_subagent_timeout_is_24_hours() {
        // Confirms the current intended default is exactly 24 h.
        const EXPECTED: u64 = 86_400;
        assert_eq!(
            SpawnTool::SUBAGENT_TIMEOUT_SECS,
            EXPECTED,
            "expected SUBAGENT_TIMEOUT_SECS to be {} (24 h), got {}",
            EXPECTED,
            SpawnTool::SUBAGENT_TIMEOUT_SECS
        );
    }
}
