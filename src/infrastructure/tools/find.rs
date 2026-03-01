// Find tool — fd-powered file discovery by glob pattern.
// Requires `fd` on PATH. Returns newline-separated relative paths.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::path_utils::resolve_to_cwd;
use crate::infrastructure::tools::truncate::format_size;

/// Default maximum number of results fd will return.
const DEFAULT_RESULT_LIMIT: usize = 1000;
/// Maximum total output bytes (50 KiB).
const MAX_OUTPUT_BYTES: usize = 50 * 1024;

pub struct FindTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
    /// Override the `fd` binary path (for testing with a dummy binary).
    fd_binary: Option<String>,
}

impl FindTool {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self {
            workspace,
            sandbox,
            fd_binary: None,
        }
    }

    /// Constructor for tests: use a custom fd binary path.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_fd_binary(
        workspace: Arc<PathBuf>,
        sandbox: Arc<Sandbox>,
        fd_binary: String,
    ) -> Self {
        Self {
            workspace,
            sandbox,
            fd_binary: Some(fd_binary),
        }
    }

    fn fd_cmd(&self) -> String {
        self.fd_binary.clone().unwrap_or_else(|| "fd".to_string())
    }
}

impl Tool for FindTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "find".to_string(),
            description: "Find files by glob pattern using fd. Requires fd on PATH. \
                          Returns newline-separated relative paths. Respects .gitignore. \
                          Output capped at 1000 results or 50KB."
                .to_string(),
            parameters_schema: r#"{
                "type": "object",
                "properties": {
                    "pattern": {"type":"string","description":"Glob pattern, e.g. '*.rs' or '**/*.json'"},
                    "path":    {"type":"string","description":"Directory to search (defaults to '.')"},
                    "limit":   {"type":"number","description":"Maximum results (default 1000)"}
                },
                "required": ["pattern"]
            }"#
            .to_string(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args_str = arguments.to_string();
        let workspace = self.workspace.clone();
        let sandbox = self.sandbox.clone();
        let fd_cmd = self.fd_cmd();

        Box::pin(async move {
            let args: serde_json::Value =
                serde_json::from_str(&args_str).map_err(|e| DomainError::Tool(e.to_string()))?;

            let pattern = args["pattern"]
                .as_str()
                .ok_or_else(|| DomainError::Tool("missing 'pattern' argument".to_string()))?;

            let search_path = args["path"].as_str().unwrap_or(".");
            let full_path = resolve_to_cwd(search_path, &workspace);
            let full_str = full_path.to_string_lossy().to_string();
            sandbox
                .validate_path(&full_str)
                .map_err(|e| DomainError::Security(e.to_string()))?;

            let limit = args["limit"]
                .as_u64()
                .map(|v| v as usize)
                .unwrap_or(DEFAULT_RESULT_LIMIT);

            // Build fd command:
            //   fd --glob --color=never --hidden --max-results N -- <pattern> <path>
            let mut cmd = tokio::process::Command::new(&fd_cmd);
            cmd.current_dir(workspace.as_ref())
                .arg("--glob")
                .arg("--color=never")
                .arg("--hidden")
                .arg("--max-results")
                .arg(limit.to_string())
                .arg("--")
                .arg(pattern)
                .arg(&full_path)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());

            let mut child = cmd.spawn().map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    DomainError::Tool(
                        "fd not found on PATH — install fd-find: https://github.com/sharkdp/fd#installation".to_string()
                    )
                } else {
                    DomainError::Tool(format!("find failed to spawn fd: {}", e))
                }
            })?;

            // Read stdout up to 2×cap before the formatter trims to MAX_OUTPUT_BYTES.
            use tokio::io::AsyncReadExt;
            let cap = MAX_OUTPUT_BYTES * 2;
            let mut stdout_bytes = Vec::with_capacity(cap.min(64 * 1024));
            if let Some(mut out) = child.stdout.take() {
                let mut buf = vec![0u8; 8192];
                loop {
                    let n = out.read(&mut buf).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    let remaining = cap.saturating_sub(stdout_bytes.len());
                    let take = n.min(remaining);
                    stdout_bytes.extend_from_slice(&buf[..take]);
                    if stdout_bytes.len() >= cap {
                        break;
                    }
                }
            }

            let stderr_bytes = {
                let mut buf = Vec::with_capacity(0);
                if let Some(mut err) = child.stderr.take() {
                    // Cap stderr at 4 KiB — enough for any error message, prevents OOM.
                    let mut tmp = vec![0u8; 4096];
                    let n = err.read(&mut tmp).await.unwrap_or(0);
                    buf.extend_from_slice(&tmp[..n]);
                }
                buf
            };

            let _ = child.kill().await;
            let status = child.wait().await;
            let exit_code = status.ok().and_then(|s| s.code());

            let stdout = String::from_utf8_lossy(&stdout_bytes);
            let stderr = String::from_utf8_lossy(&stderr_bytes);

            // fd exits 0 for matches, 1 for no matches, 2+ for errors.
            // Signal-killed (None) with no output is also an error.
            if exit_code == Some(2)
                || (exit_code.is_none() && stdout.is_empty())
                || (exit_code.is_some_and(|c| c > 2) && stdout.is_empty())
            {
                let msg = if stderr.trim().is_empty() {
                    "fd exited unexpectedly".to_string()
                } else {
                    format!("find error: {}", stderr.trim())
                };
                return Ok(ToolResult {
                    content: msg,
                    is_error: true,
                });
            }

            // Format output: relativise paths, apply byte cap, append hints.
            let result = format_find_output(&stdout, &full_path, limit, MAX_OUTPUT_BYTES);

            Ok(ToolResult {
                content: result,
                is_error: false,
            })
        })
    }
}

/// Format fd output: relativise paths to the search dir, apply byte cap, append hints.
fn format_find_output(
    raw: &str,
    search_dir: &std::path::Path,
    limit: usize,
    max_output_bytes: usize,
) -> String {
    if raw.trim().is_empty() {
        return "No files found matching pattern".to_string();
    }

    // Hoist prefix strings to avoid per-line allocation.
    let prefix = search_dir.to_string_lossy();
    let prefix_slash = format!("{}/", prefix);

    let mut output = String::new();
    let mut count = 0usize;
    let mut truncated_bytes = false;

    for line in raw.lines() {
        if line.is_empty() {
            continue;
        }
        count += 1;

        // Relativise path against the search directory.
        let rel = if let Some(rest) = line.strip_prefix(prefix_slash.as_str()) {
            rest
        } else if let Some(rest) = line.strip_prefix(prefix.as_ref()) {
            rest
        } else {
            line
        };

        if output.len() + rel.len() + 1 > max_output_bytes {
            truncated_bytes = true;
            break;
        }

        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(rel);
    }

    if output.is_empty() {
        return "No files found matching pattern".to_string();
    }

    // fd enforces --max-results so if count == limit, results were capped.
    if count >= limit && !truncated_bytes {
        output.push_str(&format!(
            "\n[{} results limit reached. Use limit={} for more, or refine pattern]",
            limit,
            limit * 2
        ));
    } else if truncated_bytes {
        let size = format_size(max_output_bytes);
        output.push_str(&format!("\n[{} limit reached]", size));
    }

    output
}

// ===========================================================================
// Unit tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    fn test_find() -> (FindTool, Arc<PathBuf>, TempDir) {
        let tmp = TempDir::new().unwrap();
        let ws = Arc::new(tmp.path().to_path_buf());
        let sandbox = Arc::new(Sandbox::new(Some(tmp.path().to_path_buf()), true));
        let tool = FindTool::new(ws.clone(), sandbox);
        (tool, ws, tmp)
    }

    // --- format_find_output unit tests ---

    fn fmt(raw: &str, dir: &str, limit: usize, cap: usize) -> String {
        format_find_output(raw, Path::new(dir), limit, cap)
    }

    #[test]
    fn test_format_find_empty() {
        assert_eq!(
            fmt("", "/ws", 1000, 50 * 1024),
            "No files found matching pattern"
        );
    }

    #[test]
    fn test_format_find_whitespace_only() {
        assert_eq!(
            fmt("   \n\n", "/ws", 1000, 50 * 1024),
            "No files found matching pattern"
        );
    }

    #[test]
    fn test_format_find_relativises_path() {
        let raw = "/ws/src/main.rs\n/ws/lib.rs";
        let result = fmt(raw, "/ws", 1000, 50 * 1024);
        assert!(result.contains("src/main.rs"), "got: {}", result);
        assert!(result.contains("lib.rs"), "got: {}", result);
        assert!(
            !result.contains("/ws/"),
            "should not contain absolute ws prefix: {}",
            result
        );
    }

    #[test]
    fn test_format_find_limit_hint() {
        // Simulate exactly `limit` lines returned — fd capped at limit.
        let lines: Vec<String> = (1..=10).map(|i| format!("/ws/file{}.rs", i)).collect();
        let raw = lines.join("\n");
        let result = fmt(&raw, "/ws", 10, 50 * 1024);
        assert!(
            result.contains("10 results limit reached"),
            "expected limit hint, got: {}",
            result
        );
    }

    #[test]
    fn test_format_find_byte_cap() {
        let lines: Vec<String> = (1..=200).map(|i| format!("/ws/file{}.txt", i)).collect();
        let raw = lines.join("\n");
        let result = fmt(&raw, "/ws", 1000, 512);
        assert!(
            result.contains("limit reached"),
            "expected byte-cap hint, got: {}",
            result
        );
    }

    #[test]
    fn test_format_find_directory_trailing_slash() {
        // fd outputs "subdir/" for directory entries — we preserve the slash.
        let raw = "/ws/subdir/";
        let result = fmt(raw, "/ws", 1000, 50 * 1024);
        assert!(result.contains("subdir/"), "got: {}", result);
    }

    // --- tool integration tests (require fd on PATH) ---

    #[tokio::test]
    async fn test_find_glob_matches() {
        let (tool, _ws, tmp) = test_find();
        std::fs::write(tmp.path().join("hello.rs"), "fn main() {}").unwrap();
        std::fs::write(tmp.path().join("notes.txt"), "notes").unwrap();

        if std::process::Command::new("fd")
            .arg("--version")
            .output()
            .is_err()
        {
            return; // fd not installed — skip
        }

        let result = tool.execute(r#"{"pattern": "*.rs"}"#).await.unwrap();
        assert!(!result.is_error, "got: {}", result.content);
        assert!(
            result.content.contains("hello.rs"),
            "got: {}",
            result.content
        );
        assert!(
            !result.content.contains("notes.txt"),
            "got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_find_no_matches() {
        let (tool, _ws, tmp) = test_find();
        std::fs::write(tmp.path().join("only.txt"), "text").unwrap();

        if std::process::Command::new("fd")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }

        let result = tool
            .execute(r#"{"pattern": "*.xyz_nonexistent"}"#)
            .await
            .unwrap();
        assert!(!result.is_error, "got: {}", result.content);
        assert!(
            result.content.contains("No files found"),
            "got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_find_outside_workspace_blocked() {
        let (tool, _ws, _tmp) = test_find();
        let result = tool
            .execute(r#"{"pattern": "*.conf", "path": "/etc"}"#)
            .await;
        assert!(result.is_err() || result.unwrap().is_error);
    }
}
