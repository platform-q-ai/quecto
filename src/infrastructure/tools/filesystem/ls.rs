// LsTool — Pi name: "ls"

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;

use super::resolve_and_validate;

/// Default maximum number of directory entries to show (Pi parity: 500).
const LS_DEFAULT_LIMIT: usize = 500;
/// Maximum allowed limit (prevents abuse).
const LS_MAX_LIMIT: usize = 5000;
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
            name: "ls".into(),
            description: format!(
                "List directory contents. Defaults to the current working directory \
                 when path is omitted. Entries are sorted case-insensitively; directories \
                 are suffixed with '/'. Output capped at {} entries or 50KB. \
                 Example: {{\"path\": \"src\"}}",
                LS_DEFAULT_LIMIT
            )
            .into(),
            parameters_schema: format!(
                r#"{{
                "type": "object",
                "properties": {{
                    "path":  {{"type":"string","description":"Directory path to list (relative or absolute, defaults to '.')"}},
                    "limit": {{"type":"number","description":"Maximum number of entries to return (default {}, max {})"}}
                }}
            }}"#,
                LS_DEFAULT_LIMIT, LS_MAX_LIMIT
            )
            .into(),
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
            // LLM-addressable: malformed JSON → Ok(is_error=true). Tool contract.
            let args: serde_json::Value = match serde_json::from_str(&args_str) {
                Ok(v) => v,
                Err(e) => return Ok(ToolResult {
                    content: format!("invalid JSON arguments: {e}. Example: {{\"path\": \".\"}}"),
                    is_error: true,
                    image_blocks: vec![],
                }),
            };

            let path = args["path"].as_str().unwrap_or(".");
            let full_path = resolve_and_validate(&workspace, &sandbox, path)?;

            // Accept both integer and float limits.
            let limit = args["limit"]
                .as_f64()
                .map(|v| (v.round() as usize).clamp(1, LS_MAX_LIMIT))
                .unwrap_or(LS_DEFAULT_LIMIT);

            let mut entries_raw = tokio::fs::read_dir(&full_path)
                .await
                .map_err(|e| DomainError::Tool(format!("ls failed: {}", e)))?;

            // Collect at most limit + 1 to detect truncation without reading the
            // entire directory (prevents OOM on huge directories).
            let read_cap = limit + 1;
            let mut names: Vec<String> = Vec::with_capacity(read_cap.min(1024));
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

            // Pi parity: empty directory message.
            let over_limit = names.len() > limit;
            if names.is_empty() {
                return Ok(ToolResult {
                    content: "(empty directory)".to_string(),
                    is_error: false,
                    image_blocks: vec![],
                });
            }

            // Pi parity: case-insensitive sort.
            // Pi parity: case-insensitive sort.
            // sort_by_key allocates the lowercase key once per entry (not per comparison).
            names.sort_by_key(|s| s.to_lowercase());

            let shown: &[String] = if over_limit { &names[..limit] } else { &names };

            // Build output and apply byte cap.
            let mut output = shown.join("\n");
            let truncated_bytes = output.len() > LS_MAX_BYTES;
            if truncated_bytes {
                let end = (0..=LS_MAX_BYTES)
                    .rev()
                    .find(|&i| output.is_char_boundary(i))
                    .unwrap_or(0);
                output.truncate(end);
            }

            // Pi parity: actionable truncation notices.
            if over_limit {
                // Suggest doubling, but cap at LS_MAX_LIMIT to avoid suggesting an impossible value.
                let suggested = (limit * 2).min(LS_MAX_LIMIT);
                output.push_str(&format!(
                    "\n[{} entries limit reached. Use limit={} for more, or use a more specific path]",
                    limit,
                    suggested
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

    // --- Pi parity ---

    #[tokio::test]
    async fn test_ls_empty_directory_message() {
        let (ws, sb, _tmp) = test_tools();
        let tool = LsTool::new(ws, sb);
        let result = tool.execute(r#"{}"#).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.content, "(empty directory)");
    }

    #[tokio::test]
    async fn test_ls_case_insensitive_sort() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("Makefile"), "").unwrap();
        std::fs::write(tmp.path().join("app.rs"), "").unwrap();
        std::fs::write(tmp.path().join("Zoo.rs"), "").unwrap();
        let tool = LsTool::new(ws, sb);
        let result = tool.execute(r#"{}"#).await.unwrap();
        assert!(!result.is_error);
        let lines: Vec<&str> = result.content.lines().collect();
        // Case-insensitive: app.rs < Makefile < Zoo.rs
        let idx_app = lines.iter().position(|&l| l == "app.rs").unwrap();
        let idx_make = lines.iter().position(|&l| l == "Makefile").unwrap();
        let idx_zoo = lines.iter().position(|&l| l == "Zoo.rs").unwrap();
        assert!(
            idx_app < idx_make && idx_make < idx_zoo,
            "expected app.rs < Makefile < Zoo.rs, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_ls_limit_parameter() {
        let (ws, sb, tmp) = test_tools();
        for i in 0..20 {
            std::fs::write(tmp.path().join(format!("file_{:04}.txt", i)), "").unwrap();
        }
        let tool = LsTool::new(ws, sb);
        let result = tool.execute(r#"{"limit": 5}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(
            result.content.contains("5 entries limit reached"),
            "expected limit notice, got: {}",
            result.content
        );
        assert!(
            result.content.contains("limit=10"),
            "expected suggested limit, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_ls_default_limit_is_500() {
        let (ws, sb, tmp) = test_tools();
        for i in 0..600 {
            std::fs::write(tmp.path().join(format!("file_{:04}.txt", i)), "").unwrap();
        }
        let tool = LsTool::new(ws, sb);
        let result = tool.execute(r#"{}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(
            result.content.contains("500 entries limit reached"),
            "expected 500 limit notice, got: {}",
            &result.content[result.content.len().saturating_sub(200)..]
        );
    }

    #[tokio::test]
    async fn test_ls_float_limit() {
        let (ws, sb, tmp) = test_tools();
        for i in 0..20 {
            std::fs::write(tmp.path().join(format!("file_{:04}.txt", i)), "").unwrap();
        }
        let tool = LsTool::new(ws, sb);
        let result = tool.execute(r#"{"limit": 5.0}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(
            result.content.contains("5 entries limit reached"),
            "expected limit notice, got: {}",
            result.content
        );
    }

    #[test]
    fn test_ls_description_includes_example() {
        let (ws, sb, _tmp) = test_tools();
        let tool = LsTool::new(ws, sb);
        let def = tool.definition();
        assert!(
            def.description.contains("Example"),
            "ls description should include Example, got: {}",
            def.description
        );
    }
}
