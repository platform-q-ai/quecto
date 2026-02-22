// Filesystem tools: read_file, write_file, edit_file, append_file, list_dir.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;

const MAX_TEXT_FILE_BYTES: u64 = 1024 * 1024;

// ===========================================================================
// Helper: resolve a relative path within the workspace and validate it.
// ===========================================================================

fn resolve_and_validate(
    workspace: &Path,
    sandbox: &Sandbox,
    relative_path: &str,
) -> Result<PathBuf, DomainError> {
    let full_path = workspace.join(relative_path);
    let full_str = full_path.to_string_lossy().to_string();
    sandbox
        .validate_path(&full_str)
        .map_err(|e| DomainError::Security(e.to_string()))
}

async fn enforce_text_file_size_limit(full_path: &Path) -> Result<(), DomainError> {
    let metadata = tokio::fs::metadata(full_path)
        .await
        .map_err(|e| DomainError::Tool(format!("metadata check failed: {}", e)))?;

    if metadata.len() > MAX_TEXT_FILE_BYTES {
        return Err(DomainError::Tool(format!(
            "file '{}' exceeds maximum allowed size ({} > {} bytes)",
            full_path.display(),
            metadata.len(),
            MAX_TEXT_FILE_BYTES
        )));
    }

    Ok(())
}

// ===========================================================================
// ReadFileTool
// ===========================================================================

pub struct ReadFileTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
}

impl ReadFileTool {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self { workspace, sandbox }
    }
}

impl Tool for ReadFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read_file".to_string(),
            description: "Read the contents of a file".to_string(),
            parameters_schema: r#"{"type":"object","properties":{"path":{"type":"string","description":"Relative path to the file"}},"required":["path"]}"#.to_string(),
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
            let path = args["path"]
                .as_str()
                .ok_or_else(|| DomainError::Tool("missing 'path' argument".to_string()))?;

            let full_path = resolve_and_validate(&workspace, &sandbox, path)?;
            enforce_text_file_size_limit(&full_path).await?;

            let content = tokio::fs::read_to_string(&full_path)
                .await
                .map_err(|e| DomainError::Tool(format!("read_file failed: {}", e)))?;

            Ok(ToolResult {
                content,
                is_error: false,
            })
        })
    }
}

// ===========================================================================
// WriteFileTool
// ===========================================================================

pub struct WriteFileTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
}

impl WriteFileTool {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self { workspace, sandbox }
    }
}

impl Tool for WriteFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write_file".to_string(),
            description: "Write content to a file (creates or overwrites)".to_string(),
            parameters_schema: r#"{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}"#.to_string(),
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
            let path = args["path"]
                .as_str()
                .ok_or_else(|| DomainError::Tool("missing 'path' argument".to_string()))?;
            let content = args["content"]
                .as_str()
                .ok_or_else(|| DomainError::Tool("missing 'content' argument".to_string()))?;

            let full_path = resolve_and_validate(&workspace, &sandbox, path)?;

            if let Some(parent) = full_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .map_err(|e| DomainError::Tool(format!("create dirs failed: {}", e)))?;
            }

            tokio::fs::write(&full_path, content)
                .await
                .map_err(|e| DomainError::Tool(format!("write_file failed: {}", e)))?;

            Ok(ToolResult {
                content: format!("wrote {} bytes to {}", content.len(), path),
                is_error: false,
            })
        })
    }
}

// ===========================================================================
// EditFileTool
// ===========================================================================

pub struct EditFileTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
}

impl EditFileTool {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self { workspace, sandbox }
    }
}

impl Tool for EditFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "edit_file".to_string(),
            description: "Edit a file by replacing a substring".to_string(),
            parameters_schema: r#"{"type":"object","properties":{"path":{"type":"string"},"old":{"type":"string"},"new":{"type":"string"}},"required":["path","old","new"]}"#.to_string(),
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
            let path = args["path"]
                .as_str()
                .ok_or_else(|| DomainError::Tool("missing 'path' argument".to_string()))?;
            let old = args["old"]
                .as_str()
                .ok_or_else(|| DomainError::Tool("missing 'old' argument".to_string()))?;
            let new = args["new"]
                .as_str()
                .ok_or_else(|| DomainError::Tool("missing 'new' argument".to_string()))?;

            let full_path = resolve_and_validate(&workspace, &sandbox, path)?;
            enforce_text_file_size_limit(&full_path).await?;

            let content = tokio::fs::read_to_string(&full_path)
                .await
                .map_err(|e| DomainError::Tool(format!("edit_file read failed: {}", e)))?;

            if !content.contains(old) {
                return Ok(ToolResult {
                    content: format!("substring '{}' not found in {}", old, path),
                    is_error: true,
                });
            }

            let updated = content.replacen(old, new, 1);
            tokio::fs::write(&full_path, &updated)
                .await
                .map_err(|e| DomainError::Tool(format!("edit_file write failed: {}", e)))?;

            Ok(ToolResult {
                content: format!("replaced '{}' with '{}' in {}", old, new, path),
                is_error: false,
            })
        })
    }
}

// ===========================================================================
// AppendFileTool
// ===========================================================================

pub struct AppendFileTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
}

impl AppendFileTool {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self { workspace, sandbox }
    }
}

impl Tool for AppendFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "append_file".to_string(),
            description: "Append content to a file".to_string(),
            parameters_schema: r#"{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}"#.to_string(),
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
            let path = args["path"]
                .as_str()
                .ok_or_else(|| DomainError::Tool("missing 'path' argument".to_string()))?;
            let content = args["content"]
                .as_str()
                .ok_or_else(|| DomainError::Tool("missing 'content' argument".to_string()))?;

            let full_path = resolve_and_validate(&workspace, &sandbox, path)?;

            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&full_path)
                .await
                .map_err(|e| DomainError::Tool(format!("append_file failed: {}", e)))?;

            file.write_all(content.as_bytes())
                .await
                .map_err(|e| DomainError::Tool(format!("append_file write failed: {}", e)))?;
            file.flush()
                .await
                .map_err(|e| DomainError::Tool(format!("append_file flush failed: {}", e)))?;

            Ok(ToolResult {
                content: format!("appended {} bytes to {}", content.len(), path),
                is_error: false,
            })
        })
    }
}

// ===========================================================================
// ListDirTool
// ===========================================================================

pub struct ListDirTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
}

impl ListDirTool {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self { workspace, sandbox }
    }
}

impl Tool for ListDirTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_dir".to_string(),
            description: "List contents of a directory".to_string(),
            parameters_schema: r#"{"type":"object","properties":{"path":{"type":"string","description":"Relative path to the directory"}},"required":["path"]}"#.to_string(),
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
            let path = args["path"]
                .as_str()
                .ok_or_else(|| DomainError::Tool("missing 'path' argument".to_string()))?;

            let full_path = resolve_and_validate(&workspace, &sandbox, path)?;

            let mut entries = tokio::fs::read_dir(&full_path)
                .await
                .map_err(|e| DomainError::Tool(format!("list_dir failed: {}", e)))?;

            let mut names: Vec<String> = Vec::new();
            while let Some(entry) = entries
                .next_entry()
                .await
                .map_err(|e| DomainError::Tool(format!("list_dir entry error: {}", e)))?
            {
                let name = entry.file_name().to_string_lossy().to_string();
                if entry
                    .file_type()
                    .await
                    .map_err(|e| DomainError::Tool(format!("list_dir file_type error: {}", e)))?
                    .is_dir()
                {
                    names.push(format!("{}/", name));
                } else {
                    names.push(name);
                }
            }

            names.sort();

            Ok(ToolResult {
                content: names.join("\n"),
                is_error: false,
            })
        })
    }
}

// ===========================================================================
// Tests
// ===========================================================================

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
    async fn test_write_and_read_file() {
        let (ws, sb, _tmp) = test_tools();
        let write_tool = WriteFileTool::new(ws.clone(), sb.clone());
        let read_tool = ReadFileTool::new(ws.clone(), sb.clone());

        let result = write_tool
            .execute(r#"{"path": "test.txt", "content": "hello world"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);

        let result = read_tool.execute(r#"{"path": "test.txt"}"#).await.unwrap();
        assert_eq!(result.content, "hello world");
    }

    #[tokio::test]
    async fn test_edit_file() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("test.txt"), "hello world").unwrap();

        let tool = EditFileTool::new(ws, sb);
        let result = tool
            .execute(r#"{"path": "test.txt", "old": "hello", "new": "goodbye"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);

        let content = std::fs::read_to_string(tmp.path().join("test.txt")).unwrap();
        assert_eq!(content, "goodbye world");
    }

    #[tokio::test]
    async fn test_edit_file_substring_not_found() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("test.txt"), "hello").unwrap();

        let tool = EditFileTool::new(ws, sb);
        let result = tool
            .execute(r#"{"path": "test.txt", "old": "xyz", "new": "abc"}"#)
            .await
            .unwrap();
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn test_append_file() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("log.txt"), "line1\n").unwrap();

        let tool = AppendFileTool::new(ws, sb);
        let result = tool
            .execute(r#"{"path": "log.txt", "content": "line2\n"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);

        let content = std::fs::read_to_string(tmp.path().join("log.txt")).unwrap();
        assert!(content.contains("line1"));
        assert!(content.contains("line2"));
    }

    #[tokio::test]
    async fn test_list_dir() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("a.txt"), "a").unwrap();
        std::fs::write(tmp.path().join("b.txt"), "b").unwrap();

        let tool = ListDirTool::new(ws, sb);
        let result = tool.execute(r#"{"path": "."}"#).await.unwrap();
        assert!(result.content.contains("a.txt"));
        assert!(result.content.contains("b.txt"));
    }

    #[tokio::test]
    async fn test_read_file_outside_workspace_blocked() {
        let (ws, sb, _tmp) = test_tools();
        let tool = ReadFileTool::new(ws, sb);
        let result = tool.execute(r#"{"path": "/etc/passwd"}"#).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_read_file_rejects_oversized_file() {
        let (ws, sb, tmp) = test_tools();
        let tool = ReadFileTool::new(ws, sb);

        let large_content = "a".repeat(1_048_577);
        std::fs::write(tmp.path().join("big.txt"), large_content).unwrap();

        let result = tool.execute(r#"{"path": "big.txt"}"#).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("exceeds maximum allowed size")
        );
    }

    #[tokio::test]
    async fn test_write_file_creates_parent_dirs() {
        let (ws, sb, tmp) = test_tools();
        let tool = WriteFileTool::new(ws, sb);
        let result = tool
            .execute(r#"{"path": "sub/dir/file.txt", "content": "nested"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(tmp.path().join("sub/dir/file.txt").exists());
    }

    #[tokio::test]
    async fn test_edit_file_rejects_oversized_file() {
        let (ws, sb, tmp) = test_tools();
        let tool = EditFileTool::new(ws, sb);

        let large_content = "a".repeat(1_048_577);
        std::fs::write(tmp.path().join("big-edit.txt"), large_content).unwrap();

        let result = tool
            .execute(r#"{"path": "big-edit.txt", "old": "a", "new": "b"}"#)
            .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("exceeds maximum allowed size")
        );
    }
}
