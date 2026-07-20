// WriteTool — tool name: "write"

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;

use super::resolve_and_validate;

pub struct WriteTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
}

impl WriteTool {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self { workspace, sandbox }
    }
}

impl Tool for WriteTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write".into(),
            description: "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Automatically creates parent directories. Example: {\"path\": \"output.txt\", \"content\": \"hello\"}".into(),
            parameters_schema: r#"{"type":"object","properties":{"path":{"type":"string","description":"Path to the file to write (relative or absolute)"},"content":{"type":"string","description":"Content to write to the file"}},"required":["path","content"]}"#.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args: Result<serde_json::Value, _> = serde_json::from_str(arguments);
        let workspace = self.workspace.clone();
        let sandbox = self.sandbox.clone();

        Box::pin(async move {
            // LLM-addressable: malformed JSON → Ok(is_error=true). Tool contract.
            let args = match args {
                Ok(v) => v,
                Err(e) => {
                    return Ok(ToolResult {
                        content: format!(
                            "invalid JSON arguments: {e}. Example: {{\"path\": \"output.txt\", \"content\": \"hello\"}}"
                        ),
                        is_error: true,
                        image_blocks: vec![],
                    });
                }
            };
            let Some(path) = args["path"].as_str() else {
                return Ok(ToolResult {
                    content:
                        "missing 'path' argument. Example: {\"path\": \"output.txt\", \"content\": \"hello\"}"
                            .to_string(),
                    is_error: true,
                    image_blocks: vec![],
                });
            };
            let Some(content) = args["content"].as_str() else {
                return Ok(ToolResult {
                    content:
                        "missing 'content' argument. Example: {\"path\": \"output.txt\", \"content\": \"hello\"}"
                            .to_string(),
                    is_error: true,
                    image_blocks: vec![],
                });
            };

            let full_path = resolve_and_validate(&workspace, &sandbox, path)?;

            if let Some(parent) = full_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| DomainError::Tool(format!("create dirs failed: {}", e)))?;
            }

            tokio::fs::write(&full_path, content)
                .await
                .map_err(|e| DomainError::Tool(format!("write failed: {}", e)))?;

            Ok(ToolResult {
                content: format!("Successfully wrote {} bytes to {}", content.len(), path),
                is_error: false,
                image_blocks: vec![],
            })
        })
    }
}

#[cfg(test)]
#[path = "write_tests.rs"]
mod tests;
