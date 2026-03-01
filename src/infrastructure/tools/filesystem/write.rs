// WriteTool — Pi name: "write" (was "write_file")

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
            name: "write".to_string(),
            description: "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Automatically creates parent directories. Example: {\"path\": \"output.txt\", \"content\": \"hello\"}".to_string(),
            parameters_schema: r#"{"type":"object","properties":{"path":{"type":"string","description":"Path to the file to write (relative or absolute)"},"content":{"type":"string","description":"Content to write to the file"}},"required":["path","content"]}"#.to_string(),
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
mod tests {
    use super::*;
    use crate::infrastructure::security::sandbox::Sandbox;
    use tempfile::TempDir;

    fn test_tools() -> (Arc<PathBuf>, Arc<Sandbox>, TempDir) {
        let tmp = TempDir::new().unwrap();
        let workspace = Arc::new(tmp.path().to_path_buf());
        let sandbox = Arc::new(Sandbox::new(Some(tmp.path().to_path_buf()), true));
        (workspace, sandbox, tmp)
    }

    #[tokio::test]
    async fn test_write_creates_parent_dirs_and_success_message() {
        let (ws, sb, tmp) = test_tools();
        let tool = WriteTool::new(ws, sb);
        let result = tool
            .execute(r#"{"path": "sub/dir/file.txt", "content": "nested"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(tmp.path().join("sub/dir/file.txt").exists());
        assert!(result.content.contains("bytes"));
    }

    #[tokio::test]
    async fn test_write_empty_object_returns_actionable_error() {
        let (ws, sb, _tmp) = test_tools();
        let tool = WriteTool::new(ws, sb);
        let result = tool.execute("{}").await.unwrap();
        assert!(result.is_error, "expected error, got: {}", result.content);
        assert!(
            result.content.contains("path"),
            "should mention 'path', got: {}",
            result.content
        );
        assert!(
            result.content.contains("Example"),
            "should include example, got: {}",
            result.content
        );
    }

    #[test]
    fn test_write_description_includes_example() {
        let (ws, sb, _tmp) = test_tools();
        let tool = WriteTool::new(ws, sb);
        let def = tool.definition();
        assert!(
            def.description.contains("Example"),
            "write description should include Example, got: {}",
            def.description
        );
    }
}
