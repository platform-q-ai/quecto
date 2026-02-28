// Filesystem tools: read, write, edit_file, append_file, list_dir.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::path_utils::{resolve_read_path, resolve_to_cwd};
use crate::infrastructure::tools::truncate::{
    DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, format_size, truncate_head,
};

// ===========================================================================
// Helper: resolve a relative path within the workspace and validate it.
// ===========================================================================

const MAX_EDIT_FILE_BYTES: u64 = 1024 * 1024;

async fn enforce_edit_file_size_limit(full_path: &Path) -> Result<(), DomainError> {
    let metadata = tokio::fs::metadata(full_path)
        .await
        .map_err(|e| DomainError::Tool(format!("metadata check failed: {}", e)))?;
    if metadata.len() > MAX_EDIT_FILE_BYTES {
        return Err(DomainError::Tool(format!(
            "file '{}' exceeds maximum allowed size for editing ({} > {} bytes)",
            full_path.display(),
            metadata.len(),
            MAX_EDIT_FILE_BYTES
        )));
    }
    Ok(())
}

fn resolve_and_validate(
    workspace: &Path,
    sandbox: &Sandbox,
    raw_path: &str,
) -> Result<PathBuf, DomainError> {
    let full_path = resolve_to_cwd(raw_path, workspace);
    let full_str = full_path.to_string_lossy().to_string();
    sandbox
        .validate_path(&full_str)
        .map_err(|e| DomainError::Security(e.to_string()))
}

// ===========================================================================
// ReadTool  (Pi name: "read")
// ===========================================================================

pub struct ReadTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
}

impl ReadTool {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self { workspace, sandbox }
    }
}

impl Tool for ReadTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read".to_string(),
            description: "Read the contents of a file. Supports text files and images (jpg, png, gif, webp). Images are sent as attachments. For text files, output is truncated to 2000 lines or 50KB (whichever is hit first). Use offset/limit for large files. When you need the full file, continue with offset until complete.".to_string(),
            parameters_schema: r#"{"type":"object","properties":{"path":{"type":"string","description":"Path to the file to read (relative or absolute)"},"offset":{"type":"number","description":"Line number to start reading from (1-indexed)"},"limit":{"type":"number","description":"Maximum number of lines to read"}},"required":["path"]}"#.to_string(),
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

            // Resolve using read-path (macOS filename variant probing)
            let resolved = resolve_read_path(path, &workspace);
            let validated_str = resolved.to_string_lossy().to_string();
            sandbox
                .validate_path(&validated_str)
                .map_err(|e| DomainError::Security(e.to_string()))?;

            // Parse optional offset (1-indexed) and limit
            let offset: Option<usize> = args["offset"].as_u64().map(|v| v as usize);
            let limit: Option<usize> = args["limit"].as_u64().map(|v| v as usize);

            // Read the full file (no size rejection — truncation handles large files)
            let content = tokio::fs::read_to_string(&resolved)
                .await
                .map_err(|e| DomainError::Tool(format!("read failed: {}", e)))?;

            let output = apply_read_truncation(&content, path, offset, limit)?;

            Ok(ToolResult {
                content: output,
                is_error: false,
            })
        })
    }
}

/// Apply offset/limit pagination and truncation to file content.
/// Returns the formatted output string with optional continuation hints.
fn apply_read_truncation(
    content: &str,
    path: &str,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<String, DomainError> {
    let total_lines: usize = if content.is_empty() {
        0
    } else {
        content.lines().count()
    };

    // Apply offset (1-indexed)
    let start_line = match offset {
        Some(0) | None => 0,
        Some(n) => {
            if n > total_lines {
                return Err(DomainError::Tool(format!(
                    "Offset {} is beyond end of file ({} lines total)",
                    n, total_lines
                )));
            }
            n - 1 // convert to 0-indexed
        }
    };

    // Build the slice from start_line
    let sliced: String = content
        .lines()
        .skip(start_line)
        .collect::<Vec<_>>()
        .join("\n");

    // Determine effective max_lines
    let max_lines = limit.unwrap_or(DEFAULT_MAX_LINES);

    // Apply head-truncation
    let tr = truncate_head(&sliced, max_lines, DEFAULT_MAX_BYTES);

    // Build output with hints
    let mut output = String::new();

    if tr.first_line_exceeds_limit {
        // Single line exceeds byte limit
        let line_size = format_size(sliced.lines().next().map_or(0, |l| l.len()));
        let limit_size = format_size(DEFAULT_MAX_BYTES);
        output.push_str(&format!(
            "[Line {} is {}, exceeds {} limit. Use bash: sed -n '{}p' {} | head -c {}]",
            start_line + 1,
            line_size,
            limit_size,
            start_line + 1,
            path,
            DEFAULT_MAX_BYTES
        ));
    } else {
        output.push_str(&tr.content);

        if tr.truncated {
            let shown_start = start_line + 1;
            let shown_end = start_line + tr.output_lines;
            let next_offset = shown_end + 1;
            let remaining = total_lines.saturating_sub(shown_end);

            if limit.is_some() && remaining > 0 {
                output.push_str(&format!(
                    "\n[{} more lines in file. Use offset={} to continue.]",
                    remaining, next_offset
                ));
            } else {
                output.push_str(&format!(
                    "\n[Showing lines {}-{} of {}. Use offset={} to continue.]",
                    shown_start, shown_end, total_lines, next_offset
                ));
            }
        }
    }

    Ok(output)
}

// ===========================================================================
// WriteFileTool
// ===========================================================================

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
            description: "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Automatically creates parent directories.".to_string(),
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
                .map_err(|e| DomainError::Tool(format!("write failed: {}", e)))?;

            Ok(ToolResult {
                content: format!("Successfully wrote {} bytes to {}", content.len(), path),
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
            enforce_edit_file_size_limit(&full_path).await?;

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
        let write_tool = WriteTool::new(ws.clone(), sb.clone());
        let read_tool = ReadTool::new(ws.clone(), sb.clone());

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
        let tool = ReadTool::new(ws, sb);
        let result = tool.execute(r#"{"path": "/etc/passwd"}"#).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_read_truncates_large_file() {
        let (ws, sb, tmp) = test_tools();
        let tool = ReadTool::new(ws, sb);

        // 3000 lines — exceeds 2000 line default limit
        let large_content: String = (1..=3000).map(|i| format!("line{}\n", i)).collect();
        std::fs::write(tmp.path().join("big.txt"), large_content).unwrap();

        let result = tool.execute(r#"{"path": "big.txt"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(
            result.content.contains("[Showing lines"),
            "expected truncation hint, got: {}",
            &result.content[..result.content.len().min(200)]
        );
    }

    #[tokio::test]
    async fn test_read_offset_pagination() {
        let (ws, sb, tmp) = test_tools();
        let tool = ReadTool::new(ws, sb);
        let content = (1..=10)
            .map(|i| format!("line{}", i))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(tmp.path().join("paged.txt"), content).unwrap();

        let result = tool
            .execute(r#"{"path": "paged.txt", "offset": 5}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("line5"), "got: {}", result.content);
        assert!(
            !result.content.contains("line4"),
            "should skip first 4 lines"
        );
    }

    #[tokio::test]
    async fn test_read_offset_beyond_eof_error() {
        let (ws, sb, tmp) = test_tools();
        let tool = ReadTool::new(ws, sb);
        std::fs::write(tmp.path().join("small.txt"), "only one line").unwrap();

        let result = tool.execute(r#"{"path": "small.txt", "offset": 99}"#).await;
        assert!(result.is_err() || result.unwrap().is_error);
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
        // Pi-parity: success message must start with "Successfully wrote"
        assert!(
            result.content.starts_with("Successfully wrote"),
            "expected Pi-format message, got: {}",
            result.content
        );
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
