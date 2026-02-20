// Spawn tool: creates an async subagent for background tasks.

use std::future::Future;
use std::pin::Pin;

use crate::domain::error::DomainError;
use crate::domain::subagent::{SubagentConfig, validate_agent_id};
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};

/// Tool that the agent can use to spawn a background subagent.
///
/// This tool validates the request and creates a SubagentConfig.
/// The actual spawning (running the subagent loop in a tokio task)
/// is the responsibility of the caller (gateway/agent orchestrator).
#[derive(Debug)]
pub struct SpawnTool {
    /// Allowlist of agent IDs that can be spawned.
    allowed_agents: Vec<String>,
    /// Whether workspace restriction should be inherited.
    restrict_to_workspace: bool,
}

impl SpawnTool {
    pub fn new(allowed_agents: Vec<String>, restrict_to_workspace: bool) -> Self {
        Self {
            allowed_agents,
            restrict_to_workspace,
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

        let deliver_to = args
            .get("deliver_to")
            .and_then(|v| v.as_str())
            .map(String::from);

        // Validate agent_id if provided and allowlist is non-empty
        if let Some(ref id) = agent_id
            && !self.allowed_agents.is_empty()
        {
            validate_agent_id(id, &self.allowed_agents).map_err(|e| e.to_string())?;
        }

        Ok(SubagentConfig {
            task,
            agent_id,
            restrict_to_workspace: self.restrict_to_workspace,
            deliver_to,
        })
    }
}

impl Tool for SpawnTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "spawn".to_string(),
            description: "Spawn a background subagent to handle a long-running task asynchronously"
                .to_string(),
            parameters_schema: r#"{"type":"object","properties":{"task":{"type":"string","description":"The task description for the subagent"},"agent_id":{"type":"string","description":"Optional agent ID to spawn (must be in allowlist)"},"deliver_to":{"type":"string","description":"Optional channel:chat_id to deliver results to"}},"required":["task"]}"#.to_string(),
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
                    let msg = format!(
                        "Subagent spawned for task: '{}'. Restrict to workspace: {}.",
                        config.task, config.restrict_to_workspace
                    );
                    Ok(ToolResult {
                        content: msg,
                        is_error: false,
                    })
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

    #[tokio::test]
    async fn test_spawn_valid_task() {
        let tool = test_tool();
        let result = tool.execute(r#"{"task":"Summarize news"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Summarize news"));
    }

    #[tokio::test]
    async fn test_spawn_with_agent_id() {
        let tool = test_tool();
        let result = tool
            .execute(r#"{"task":"Get weather","agent_id":"weather-bot"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn test_spawn_disallowed_agent() {
        let tool = test_tool();
        let result = tool
            .execute(r#"{"task":"Evil task","agent_id":"evil-bot"}"#)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not allowed"));
    }

    #[tokio::test]
    async fn test_spawn_missing_task() {
        let tool = test_tool();
        let result = tool.execute(r#"{"agent_id":"news-bot"}"#).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("missing"));
    }

    #[tokio::test]
    async fn test_spawn_empty_allowlist_permits_any() {
        let tool = SpawnTool::new(vec![], true);
        let result = tool
            .execute(r#"{"task":"Do stuff","agent_id":"any-bot"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
    }
}
