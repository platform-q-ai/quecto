// Grep tool — ripgrep-powered file content search.
// Requires `rg` on PATH. Returns structured match output with file:line:content format.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::path_utils::resolve_to_cwd;
use crate::infrastructure::tools::truncate::format_size;

/// Maximum number of matches to return (default).
const DEFAULT_MATCH_LIMIT: usize = 100;
/// Maximum line length before truncation.
const MAX_LINE_BYTES: usize = 500;
/// Maximum total output bytes.
const MAX_OUTPUT_BYTES: usize = 50 * 1024;

pub struct GrepTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
    /// Override the `rg` binary path (for testing with a dummy binary).
    rg_binary: Option<String>,
}

impl GrepTool {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self {
            workspace,
            sandbox,
            rg_binary: None,
        }
    }

    /// Constructor for tests: use a custom rg binary path.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_rg_binary(
        workspace: Arc<PathBuf>,
        sandbox: Arc<Sandbox>,
        rg_binary: String,
    ) -> Self {
        Self {
            workspace,
            sandbox,
            rg_binary: Some(rg_binary),
        }
    }

    fn rg_cmd(&self) -> String {
        self.rg_binary.clone().unwrap_or_else(|| "rg".to_string())
    }
}

impl Tool for GrepTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "grep".to_string(),
            description: "Search file contents using ripgrep (rg). Requires rg on PATH. \
                          Returns file:line:content matches. Output capped at 100 matches or 50KB."
                .to_string(),
            parameters_schema: r#"{
                "type": "object",
                "properties": {
                    "pattern":    {"type":"string","description":"Search pattern (regex or literal)"},
                    "path":       {"type":"string","description":"Directory or file to search (defaults to '.')"},
                    "glob":       {"type":"string","description":"Glob pattern to filter files, e.g. '*.rs'"},
                    "ignoreCase": {"type":"boolean","description":"Case-insensitive search"},
                    "literal":    {"type":"boolean","description":"Treat pattern as literal string"},
                    "context":    {"type":"number","description":"Context lines before and after each match"},
                    "limit":      {"type":"number","description":"Maximum matches to return (default 100)"}
                },
                "required": ["pattern"]
            }"#
            .to_string(),
        }
    }

    #[allow(clippy::too_many_lines)]
    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args_str = arguments.to_string();
        let workspace = self.workspace.clone();
        let sandbox = self.sandbox.clone();
        let rg_cmd = self.rg_cmd();

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

            let glob = args["glob"].as_str();
            let ignore_case = args["ignoreCase"].as_bool().unwrap_or(false);
            let literal = args["literal"].as_bool().unwrap_or(false);
            let context_lines = args["context"].as_u64().unwrap_or(0) as usize;
            let limit = args["limit"]
                .as_u64()
                .map(|v| v as usize)
                .unwrap_or(DEFAULT_MATCH_LIMIT);

            // Build rg command
            let mut cmd = tokio::process::Command::new(&rg_cmd);
            cmd.current_dir(workspace.as_ref())
                .arg("--line-number")
                .arg("--color=never")
                .arg("--hidden")
                .arg("--no-heading");

            if ignore_case {
                cmd.arg("--ignore-case");
            }
            if literal {
                cmd.arg("--fixed-strings");
            }
            if let Some(g) = glob {
                cmd.arg("--glob").arg(g);
            }
            if context_lines > 0 {
                cmd.arg(format!("--context={}", context_lines));
            }

            cmd.arg("--").arg(pattern).arg(&full_path);
            // Pipe stdout so we can cap bytes before buffering into a String.
            cmd.stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());

            // Spawn rg and read output, capping at MAX_OUTPUT_BYTES * 2 to avoid OOM
            // on adversarial inputs (the formatter will further trim to MAX_OUTPUT_BYTES).
            let cap = MAX_OUTPUT_BYTES * 2;
            let mut child = cmd.spawn().map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    DomainError::Tool(
                        "rg not found on PATH — install ripgrep: https://github.com/BurntSushi/ripgrep#installation".to_string()
                    )
                } else {
                    DomainError::Tool(format!("grep failed to spawn rg: {}", e))
                }
            })?;

            // Read stdout up to cap, then kill rg (match limit already handles this
            // for normal use, but cap protects against unexpectedly large output).
            use tokio::io::AsyncReadExt;
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

            let stderr_output = {
                let mut buf = Vec::with_capacity(0);
                if let Some(mut err) = child.stderr.take() {
                    // Cap stderr at 4 KiB — enough for any error message, prevents OOM.
                    let mut tmp = vec![0u8; 4096];
                    let n = err.read(&mut tmp).await.unwrap_or(0);
                    buf.extend_from_slice(&tmp[..n]);
                }
                buf
            };

            // Reap child (ignore errors — process may already have exited naturally).
            let _ = child.kill().await;
            let status = child.wait().await;

            // rg exits: 0 = matches found, 1 = no matches, 2+ = error, None = signal-killed.
            let exit_code = status.ok().and_then(|s| s.code());
            let stdout = String::from_utf8_lossy(&stdout_bytes);
            let stderr = String::from_utf8_lossy(&stderr_output);

            // Exit code 2+ (or signal-killed with no output) = rg error
            if exit_code == Some(2)
                || (exit_code.is_none() && stdout.is_empty())
                || (exit_code.is_some_and(|c| c > 2) && stdout.is_empty())
            {
                let msg = if stderr.trim().is_empty() {
                    "rg exited unexpectedly".to_string()
                } else {
                    format!("grep error: {}", stderr.trim())
                };
                return Ok(ToolResult {
                    content: msg,
                    is_error: true,
                    image_blocks: vec![],
                });
            }

            // Parse lines and apply truncation
            let result = format_grep_output(GrepOutputArgs {
                raw: &stdout,
                workspace: &workspace,
                match_limit: limit,
                max_line_bytes: MAX_LINE_BYTES,
                max_output_bytes: MAX_OUTPUT_BYTES,
            });

            Ok(ToolResult {
                content: result,
                is_error: false,
                image_blocks: vec![],
            })
        })
    }
}

struct GrepOutputArgs<'a> {
    raw: &'a str,
    workspace: &'a Path,
    match_limit: usize,
    max_line_bytes: usize,
    max_output_bytes: usize,
}

/// Format rg output lines, applying match limit and byte truncation.
/// Each line from rg is: `file:line:content` for matches, `--` for separators.
fn format_grep_output(a: GrepOutputArgs<'_>) -> String {
    let GrepOutputArgs {
        raw,
        workspace,
        match_limit,
        max_line_bytes,
        max_output_bytes,
    } = a;
    if raw.trim().is_empty() {
        return "No matches found".to_string();
    }

    // Hoist prefix computation out of the per-line loop.
    let ws_prefix = workspace.to_string_lossy();
    let ws_prefix_slash = format!("{}/", ws_prefix);

    let mut output = String::new();
    let mut match_count = 0usize;
    let mut truncated_limit = false;
    let mut truncated_bytes = false;

    for line in raw.lines() {
        // Each rg line is "path:lineno:content" for matches, "path-lineno-context" for context,
        // or "--" for group separators.
        let is_match_line = is_rg_match_line(line);

        if is_match_line {
            if match_count >= match_limit {
                truncated_limit = true;
                break;
            }
            match_count += 1;
        }

        // Truncate long lines (fast path: skip allocation when short).
        let display_line = truncate_line(line, max_line_bytes);

        // Make paths relative to workspace (fast path: only strip known prefix).
        let display_line = relativise_with_prefix(&display_line, &ws_prefix, &ws_prefix_slash);

        if output.len() + display_line.len() + 1 > max_output_bytes {
            truncated_bytes = true;
            break;
        }

        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&display_line);
    }

    if output.is_empty() {
        return "No matches found".to_string();
    }

    // Append truncation hints
    if truncated_limit {
        output.push_str(&format!(
            "\n[Showing first {} matches. Use a more specific pattern or path to narrow results.]",
            match_limit
        ));
    } else if truncated_bytes {
        let size = format_size(max_output_bytes);
        output.push_str(&format!("\n[Output truncated at {}]", size));
    }

    output
}

/// Detect whether a rg output line is a match line (file:lineno:content) vs separator/context.
fn is_rg_match_line(line: &str) -> bool {
    // Match lines have format: path:N:content (colon separators, N is numeric)
    // Context lines have: path-N-content
    // Separator lines: "--"
    if line == "--" {
        return false;
    }
    // Try to find colon-separated triplet where second field is numeric
    let mut parts = line.splitn(3, ':');
    if parts.next().is_some() {
        if let Some(lineno) = parts.next() {
            return lineno.parse::<u64>().is_ok();
        }
    }
    false
}

/// Truncate a line to max_bytes, appending a size hint if truncated.
fn truncate_line(line: &str, max_bytes: usize) -> String {
    if line.len() <= max_bytes {
        return line.to_string();
    }
    // Find char boundary at max_bytes
    let end = (0..=max_bytes)
        .rev()
        .find(|&i| line.is_char_boundary(i))
        .unwrap_or(0);
    let size_hint = format_size(line.len());
    format!("{}… [line is {}]", &line[..end], size_hint)
}

/// Replace absolute workspace prefix with relative path in the line.
/// Accepts pre-computed prefix strings to avoid repeated allocations in hot loop.
fn relativise_with_prefix<'a>(
    line: &'a str,
    prefix: &str,
    prefix_slash: &str,
) -> std::borrow::Cow<'a, str> {
    if let Some(rest) = line.strip_prefix(prefix_slash) {
        std::borrow::Cow::Owned(rest.to_string())
    } else if let Some(rest) = line.strip_prefix(prefix) {
        std::borrow::Cow::Owned(rest.to_string())
    } else {
        std::borrow::Cow::Borrowed(line)
    }
}

// ===========================================================================
// Unit tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_grep() -> (GrepTool, Arc<PathBuf>, TempDir) {
        let tmp = TempDir::new().unwrap();
        let ws = Arc::new(tmp.path().to_path_buf());
        // restrict_to_workspace: true — sandbox enforces workspace containment
        let sandbox = Arc::new(Sandbox::new(Some(tmp.path().to_path_buf()), true));
        let tool = GrepTool::new(ws.clone(), sandbox);
        (tool, ws, tmp)
    }

    #[allow(clippy::too_many_arguments)]
    fn grep_output(raw: &str, ws: &str, limit: usize, line: usize, bytes: usize) -> String {
        format_grep_output(GrepOutputArgs {
            raw,
            workspace: &PathBuf::from(ws),
            match_limit: limit,
            max_line_bytes: line,
            max_output_bytes: bytes,
        })
    }

    #[test]
    fn test_format_grep_output_empty() {
        assert_eq!(
            grep_output("", "/ws", 100, 500, 50 * 1024),
            "No matches found"
        );
    }

    #[test]
    fn test_format_grep_output_basic() {
        let raw = "/ws/main.rs:1:fn main() {}";
        let result = grep_output(raw, "/ws", 100, 500, 50 * 1024);
        assert!(result.contains("main.rs:1:"));
    }

    #[test]
    fn test_format_grep_output_relativises_path() {
        let raw = "/workspace/src/foo.rs:5:hello world";
        let result = grep_output(raw, "/workspace", 100, 500, 50 * 1024);
        assert!(result.contains("src/foo.rs:5:hello world"));
        assert!(!result.contains("/workspace/src/foo.rs"));
    }

    #[test]
    fn test_format_grep_output_match_limit() {
        let lines: Vec<String> = (1..=150)
            .map(|i| format!("/ws/file.rs:{}:needle here", i))
            .collect();
        let raw = lines.join("\n");
        let result = grep_output(&raw, "/ws", 10, 500, 50 * 1024);
        assert!(
            result.contains("[Showing first 10 matches"),
            "expected limit hint, got: {}",
            &result[..result.len().min(200)]
        );
    }

    #[test]
    fn test_format_grep_output_byte_limit() {
        // Make lines that individually fit but collectively exceed 1KB output cap
        let lines: Vec<String> = (1..=200)
            .map(|i| format!("/ws/f.rs:{}:{}", i, "x".repeat(20)))
            .collect();
        let raw = lines.join("\n");
        let result = grep_output(&raw, "/ws", 1000, 500, 1024);
        assert!(
            result.contains("[Output truncated"),
            "expected byte truncation, got: {}",
            &result[..result.len().min(200)]
        );
    }

    #[test]
    fn test_truncate_line_short() {
        let result = truncate_line("hello", 500);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_truncate_line_long() {
        let long = "x".repeat(600);
        let result = truncate_line(&long, 500);
        assert!(result.contains("…"), "expected ellipsis");
        assert!(result.len() < 600);
    }

    #[test]
    fn test_is_rg_match_line() {
        assert!(is_rg_match_line("src/foo.rs:42:hello world"));
        assert!(!is_rg_match_line("--"));
        assert!(!is_rg_match_line("src/foo.rs-42-context line"));
        assert!(!is_rg_match_line("not-a-match-line"));
    }

    #[test]
    fn test_relativise_path() {
        let prefix = "/workspace";
        let prefix_slash = "/workspace/";
        assert_eq!(
            relativise_with_prefix("/workspace/src/main.rs:1:hello", prefix, prefix_slash),
            "src/main.rs:1:hello"
        );
        assert_eq!(
            relativise_with_prefix("/other/path:1:hello", prefix, prefix_slash),
            "/other/path:1:hello"
        );
    }

    #[tokio::test]
    async fn test_grep_finds_pattern() {
        let (tool, _ws, tmp) = test_grep();
        std::fs::write(
            tmp.path().join("hello.rs"),
            "fn hello() { println!(\"hi\"); }\n",
        )
        .unwrap();

        // Skip if rg not available
        if std::process::Command::new("rg")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }

        let result = tool.execute(r#"{"pattern": "hello"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("hello"), "got: {}", result.content);
    }

    #[tokio::test]
    async fn test_grep_no_matches() {
        let (tool, _ws, tmp) = test_grep();
        std::fs::write(tmp.path().join("file.rs"), "fn nothing() {}\n").unwrap();

        if std::process::Command::new("rg")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }

        let result = tool
            .execute(r#"{"pattern": "xyz_nonexistent_9999"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(
            result.content.contains("No matches found"),
            "got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_grep_outside_workspace_blocked() {
        let (tool, _ws, _tmp) = test_grep();
        let result = tool.execute(r#"{"pattern": "root", "path": "/etc"}"#).await;
        assert!(result.is_err() || result.unwrap().is_error);
    }
}
