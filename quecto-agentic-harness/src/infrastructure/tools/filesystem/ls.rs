// LsTool — tool name: "ls"

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;

use super::resolve_and_validate;

/// Default maximum number of directory entries to show (Quecto compatibility: 500).
const LS_DEFAULT_LIMIT: usize = 500;
/// Maximum allowed limit (prevents abuse).
const LS_MAX_LIMIT: usize = 5000;
/// Maximum output bytes before truncating.
const LS_MAX_BYTES: usize = crate::domain::constants::DEFAULT_OUTPUT_CAP_BYTES;

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
                Err(e) => {
                    return Ok(ToolResult {
                        content: format!(
                            "invalid JSON arguments: {e}. Example: {{\"path\": \".\"}}"
                        ),
                        is_error: true,
                        image_blocks: vec![],
                    });
                }
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

            // Quecto compatibility: empty directory message.
            let over_limit = names.len() > limit;
            if names.is_empty() {
                return Ok(ToolResult {
                    content: "(empty directory)".to_string(),
                    is_error: false,
                    image_blocks: vec![],
                });
            }

            // Quecto compatibility: case-insensitive sort.
            // Quecto compatibility: case-insensitive sort.
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

            // Quecto compatibility: actionable truncation notices.
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
#[path = "ls_tests.rs"]
mod tests;
