// LsTool — Pi name: "ls" (was "list_dir")

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;

use super::resolve_and_validate;

/// Maximum number of directory entries to show before truncating.
const LS_MAX_ENTRIES: usize = 1000;
/// Maximum output bytes before truncating.
const LS_MAX_BYTES: usize = 50 * 1024;

pub struct LsTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
}

impl LsTool {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self { workspace, sandbox }
    }
}

impl Tool for LsTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "ls".to_string(),
            description: "List directory contents. Defaults to the current working directory \
                          when path is omitted. Entries are sorted; directories are suffixed with \
                          '/'. Output is capped at 1000 entries or 50KB."
                .to_string(),
            parameters_schema: r#"{"type":"object","properties":{"path":{"type":"string","description":"Directory path to list (relative or absolute, defaults to '.')"}}}"#.to_string(),
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

            let path = args["path"].as_str().unwrap_or(".");
            let full_path = resolve_and_validate(&workspace, &sandbox, path)?;

            let mut entries_raw = tokio::fs::read_dir(&full_path)
                .await
                .map_err(|e| DomainError::Tool(format!("ls failed: {}", e)))?;

            // Collect at most LS_MAX_ENTRIES + 1 to detect truncation without reading
            // the entire directory (prevents OOM on huge directories).
            let read_cap = LS_MAX_ENTRIES + 1;
            let mut names: Vec<String> = Vec::with_capacity(read_cap);
            loop {
                if names.len() >= read_cap {
                    break;
                }
                match entries_raw
                    .next_entry()
                    .await
                    .map_err(|e| DomainError::Tool(format!("ls entry error: {}", e)))?
                {
                    None => break,
                    Some(entry) => {
                        let name = entry.file_name().to_string_lossy().to_string();
                        let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
                        if is_dir {
                            names.push(format!("{}/", name));
                        } else {
                            names.push(name);
                        }
                    }
                }
            }

            let over_limit = names.len() > LS_MAX_ENTRIES;
            names.sort();
            let shown: &[String] = if over_limit {
                &names[..LS_MAX_ENTRIES]
            } else {
                &names
            };

            // Build output and apply byte cap — scan backward to stay under cap
            let mut output = shown.join("\n");
            let truncated_bytes = output.len() > LS_MAX_BYTES;
            if truncated_bytes {
                let end = (0..=LS_MAX_BYTES)
                    .rev()
                    .find(|&i| output.is_char_boundary(i))
                    .unwrap_or(0);
                output.truncate(end);
            }

            if over_limit {
                output.push_str(&format!(
                    "\n[Showing {} of >={} entries. Use a more specific path to see more.]",
                    LS_MAX_ENTRIES, LS_MAX_ENTRIES
                ));
            } else if truncated_bytes {
                output.push_str(&format!("\n[Output truncated at {} bytes]", LS_MAX_BYTES));
            }

            Ok(ToolResult {
                content: output,
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
    async fn test_ls_lists_files_and_dirs() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("a.txt"), "").unwrap();
        std::fs::create_dir(tmp.path().join("subdir")).unwrap();
        let tool = LsTool::new(ws, sb);
        let result = tool.execute(r#"{}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("a.txt"));
        assert!(result.content.contains("subdir/"));
    }

    #[tokio::test]
    async fn test_ls_defaults_to_workspace() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("file.txt"), "").unwrap();
        let tool = LsTool::new(ws, sb);
        // No path argument
        let result = tool.execute(r#"{}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("file.txt"));
    }

    #[tokio::test]
    async fn test_ls_subdirectory() {
        let (ws, sb, tmp) = test_tools();
        std::fs::create_dir(tmp.path().join("sub")).unwrap();
        std::fs::write(tmp.path().join("sub/inner.txt"), "x").unwrap();
        let tool = LsTool::new(ws, sb);
        let result = tool.execute(r#"{"path": "sub"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("inner.txt"));
    }

    #[tokio::test]
    async fn test_ls_blocks_path_traversal() {
        let (ws, sb, _tmp) = test_tools();
        let tool = LsTool::new(ws, sb);
        let result = tool.execute(r#"{"path": "/etc"}"#).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_ls_truncates_at_1000_entries() {
        let (ws, sb, tmp) = test_tools();
        // Create 1001 files
        for i in 0..=1000 {
            std::fs::write(tmp.path().join(format!("file{:04}.txt", i)), "").unwrap();
        }
        let tool = LsTool::new(ws, sb);
        let result = tool.execute(r#"{}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(
            result.content.contains("[Showing 1000 of >=1000 entries"),
            "expected truncation hint, got: {}",
            &result.content[result.content.len().saturating_sub(200)..]
        );
    }
}
