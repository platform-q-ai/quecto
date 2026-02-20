// Shell execution tool: impl Tool for ExecTool.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;

/// Tool that executes shell commands within the workspace.
pub struct ExecTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
}

impl ExecTool {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self { workspace, sandbox }
    }
}

impl Tool for ExecTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "exec".to_string(),
            description: "Execute a shell command in the workspace directory".to_string(),
            parameters_schema: r#"{"type":"object","properties":{"command":{"type":"string","description":"Shell command to execute"}},"required":["command"]}"#.to_string(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args_str = arguments.to_string();
        let workspace = self.workspace.clone();
        let sandbox = self.sandbox.clone();

        Box::pin(async move {
            let args: serde_json::Value =
                serde_json::from_str(&args_str).map_err(|e| DomainError::Tool(e.to_string()))?;

            let command = args["command"]
                .as_str()
                .ok_or_else(|| DomainError::Tool("missing 'command' argument".to_string()))?;

            // Validate command against dangerous patterns
            sandbox
                .validate_command(command)
                .map_err(|e| DomainError::Security(e.to_string()))?;

            let output = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(command)
                .current_dir(workspace.as_ref())
                .output()
                .await
                .map_err(|e| DomainError::Tool(format!("exec failed: {}", e)))?;

            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            if output.status.success() {
                Ok(ToolResult {
                    content: stdout.to_string(),
                    is_error: false,
                })
            } else {
                Ok(ToolResult {
                    content: format!(
                        "exit code {}\nstdout: {}\nstderr: {}",
                        output.status, stdout, stderr
                    ),
                    is_error: true,
                })
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_exec(restrict: bool) -> (ExecTool, TempDir) {
        let tmp = TempDir::new().unwrap();
        let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), restrict);
        let tool = ExecTool::new(Arc::new(tmp.path().to_path_buf()), Arc::new(sandbox));
        (tool, tmp)
    }

    #[tokio::test]
    async fn test_exec_echo() {
        let (tool, _tmp) = test_exec(false);
        let result = tool.execute(r#"{"command": "echo hello"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("hello"));
    }

    #[tokio::test]
    async fn test_exec_dangerous_command_blocked() {
        let (tool, _tmp) = test_exec(false);
        let result = tool.execute(r#"{"command": "rm -rf /"}"#).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_exec_missing_command_arg() {
        let (tool, _tmp) = test_exec(false);
        let result = tool.execute(r#"{}"#).await;
        assert!(result.is_err());
    }
}
