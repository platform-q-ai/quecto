// Grep tool — ripgrep-powered file content search (Pi parity).
// Uses `rg --json` for robust structured match extraction.
// Context lines are extracted from a file cache, not rg's --context output.

use std::collections::HashMap;
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
/// Maximum line length before truncation (chars); matches Pi's GREP_MAX_LINE_LENGTH.
const MAX_LINE_BYTES: usize = 500;
/// Maximum total output bytes (50KB); matches Pi's DEFAULT_MAX_BYTES.
const MAX_OUTPUT_BYTES: usize = 50 * 1024;
/// Maximum individual file size for context reads (1MB); prevents OOM from huge cached files.
const MAX_FILE_CACHE_BYTES: usize = 1024 * 1024;
/// Maximum context lines per side; prevents unbounded file reads.
const MAX_CONTEXT_LINES: usize = 50;

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
            description: format!(
                "Search file contents using ripgrep (rg). Requires rg on PATH. \
                 Returns file:line:content matches with optional context lines (file-N- format). \
                 Output capped at {} matches or {}KB. \
                 Example: {{\"pattern\": \"search_term\"}}",
                DEFAULT_MATCH_LIMIT,
                MAX_OUTPUT_BYTES / 1024
            ),
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

            let Some(pattern) = args["pattern"].as_str() else {
                return Ok(ToolResult {
                    content: "missing 'pattern' argument. Example: {\"pattern\": \"search_term\"}"
                        .to_string(),
                    is_error: true,
                    image_blocks: vec![],
                });
            };

            let search_path = args["path"].as_str().unwrap_or(".");
            let full_path = resolve_to_cwd(search_path, &workspace);
            let full_str = full_path.to_string_lossy().to_string();
            sandbox
                .validate_path(&full_str)
                .map_err(|e| DomainError::Security(e.to_string()))?;

            let glob = args["glob"].as_str().map(String::from);
            let ignore_case = args["ignoreCase"].as_bool().unwrap_or(false);
            let literal = args["literal"].as_bool().unwrap_or(false);
            let context_lines =
                (args["context"].as_f64().unwrap_or(0.0) as usize).min(MAX_CONTEXT_LINES);
            let limit = args["limit"]
                .as_f64()
                .map(|v| (v.round() as usize).max(1))
                .unwrap_or(DEFAULT_MATCH_LIMIT);

            let cmd = build_rg_command(RgArgs {
                rg_cmd: &rg_cmd,
                workspace: &workspace,
                pattern,
                full_path: &full_path,
                glob: glob.as_deref(),
                ignore_case,
                literal,
            });
            let (stdout_bytes, stderr_bytes, exit_code) = run_rg(cmd).await?;

            // rg exits: 0 = matches found, 1 = no matches, 2+ = error, None = signal-killed.
            // Exit code 2 always indicates an error, regardless of stdout content.
            let is_rg_error = exit_code == Some(2)
                || (exit_code.is_none() && stdout_bytes.is_empty())
                || exit_code.is_some_and(|c| c > 2);

            if is_rg_error {
                let stderr = String::from_utf8_lossy(&stderr_bytes);
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

            let stdout = String::from_utf8_lossy(&stdout_bytes);
            let result = format_grep_output(GrepFormatArgs {
                json_output: &stdout,
                workspace: &workspace,
                sandbox: &sandbox,
                match_limit: limit,
                context_lines,
                max_line_bytes: MAX_LINE_BYTES,
                max_output_bytes: MAX_OUTPUT_BYTES,
            })
            .await;

            Ok(ToolResult {
                content: result,
                is_error: false,
                image_blocks: vec![],
            })
        })
    }
}

// ---------------------------------------------------------------------------
// rg invocation
// ---------------------------------------------------------------------------

struct RgArgs<'a> {
    rg_cmd: &'a str,
    workspace: &'a Path,
    pattern: &'a str,
    full_path: &'a Path,
    glob: Option<&'a str>,
    ignore_case: bool,
    literal: bool,
}

/// Build the ripgrep command. Uses `--json` for structured output.
/// Context lines are extracted from a file cache after parsing, not via `--context`.
fn build_rg_command(a: RgArgs<'_>) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(a.rg_cmd);
    cmd.current_dir(a.workspace)
        .arg("--json")
        .arg("--line-number")
        .arg("--color=never")
        .arg("--hidden");
    if a.ignore_case {
        cmd.arg("--ignore-case");
    }
    if a.literal {
        cmd.arg("--fixed-strings");
    }
    if let Some(g) = a.glob {
        cmd.arg("--glob").arg(g);
    }
    cmd.arg("--").arg(a.pattern).arg(a.full_path);
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    cmd
}

/// Spawn rg, read capped stdout/stderr, reap child.
async fn run_rg(
    mut cmd: tokio::process::Command,
) -> Result<(Vec<u8>, Vec<u8>, Option<i32>), DomainError> {
    use tokio::io::AsyncReadExt;

    let cap = MAX_OUTPUT_BYTES * 4; // JSON is larger than plain text
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            DomainError::Tool(
                "rg not found on PATH — install ripgrep: https://github.com/BurntSushi/ripgrep#installation".to_string()
            )
        } else {
            DomainError::Tool(format!("grep failed to spawn rg: {}", e))
        }
    })?;

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

    let mut stderr_bytes = Vec::new();
    if let Some(mut err) = child.stderr.take() {
        let mut tmp = vec![0u8; 4096];
        let n = err.read(&mut tmp).await.unwrap_or(0);
        stderr_bytes.extend_from_slice(&tmp[..n]);
    }

    let _ = child.kill().await;
    let status = child.wait().await;
    let exit_code = status.ok().and_then(|s| s.code());
    Ok((stdout_bytes, stderr_bytes, exit_code))
}

// ---------------------------------------------------------------------------
// JSON parsing and output formatting
// ---------------------------------------------------------------------------

/// A parsed ripgrep match event.
struct RgMatch {
    /// Absolute file path.
    file_path: PathBuf,
    /// 1-based line number of the match.
    line_number: usize,
}

/// Parse `rg --json` output: extract only `"match"` type events.
fn parse_rg_matches(json_output: &str) -> Vec<RgMatch> {
    let mut matches = Vec::new();
    for line in json_output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if event["type"].as_str() != Some("match") {
            continue;
        }
        let Some(file_path) = event["data"]["path"]["text"].as_str() else {
            continue;
        };
        let Some(line_number) = event["data"]["line_number"].as_u64() else {
            continue;
        };
        matches.push(RgMatch {
            file_path: PathBuf::from(file_path),
            line_number: line_number as usize,
        });
    }
    matches
}

struct GrepFormatArgs<'a> {
    json_output: &'a str,
    workspace: &'a Path,
    sandbox: &'a Sandbox,
    match_limit: usize,
    context_lines: usize,
    max_line_bytes: usize,
    max_output_bytes: usize,
}

/// Format parsed matches with file-cache-based context extraction (Pi parity).
/// Configuration shared across all match blocks during formatting.
struct BlockConfig<'a> {
    ws_str: &'a str,
    ws_prefix_slash: &'a str,
    context_lines: usize,
    max_line_bytes: usize,
    max_output_bytes: usize,
}

/// State accumulated while formatting matches.
struct FormatState {
    output_lines: Vec<String>,
    byte_total: usize,
    lines_truncated: bool,
    truncated_bytes: bool,
}

/// Format one match block (match line + optional context lines) into `state`.
/// Returns `false` when the byte limit is exceeded and formatting should stop.
fn format_match_block(
    m: &RgMatch,
    file_cache: &mut HashMap<PathBuf, Vec<String>>,
    cfg: &BlockConfig<'_>,
    state: &mut FormatState,
) -> bool {
    let file_lines = file_cache
        .entry(m.file_path.clone())
        .or_insert_with(|| read_file_for_cache(&m.file_path));

    let raw_path = m.file_path.to_string_lossy();
    let rel_path = if let Some(rest) = raw_path.strip_prefix(cfg.ws_prefix_slash) {
        rest
    } else if let Some(rest) = raw_path.strip_prefix(cfg.ws_str) {
        rest
    } else {
        raw_path.as_ref()
    };

    let total_lines = file_lines.len();
    let start = if cfg.context_lines > 0 {
        m.line_number.saturating_sub(cfg.context_lines).max(1)
    } else {
        m.line_number
    };
    let end = if cfg.context_lines > 0 {
        (m.line_number + cfg.context_lines).min(total_lines)
    } else {
        m.line_number
    };

    for current in start..=end {
        let line_text = file_lines
            .get(current - 1)
            .map(String::as_str)
            .unwrap_or("");
        let sanitized = line_text.trim_end_matches('\n');
        let (display_text, was_truncated) = truncate_line(sanitized, cfg.max_line_bytes);
        if was_truncated {
            state.lines_truncated = true;
        }

        let formatted = if current == m.line_number {
            format!("{}:{}: {}", rel_path, current, display_text)
        } else {
            format!("{}-{}- {}", rel_path, current, display_text)
        };

        state.byte_total += formatted.len() + 1;
        if state.byte_total > cfg.max_output_bytes {
            state.truncated_bytes = true;
            return false;
        }
        state.output_lines.push(formatted);
    }
    true
}

async fn format_grep_output(a: GrepFormatArgs<'_>) -> String {
    let all_matches = parse_rg_matches(a.json_output);
    // Detect limit exceeded: true only when rg returned MORE than the limit.
    // When rg returns exactly `match_limit` matches with no more available, we
    // do NOT show the limit notice (avoid false-positive "limit reached").
    let total_match_count = all_matches.len();
    let capped: Vec<_> = all_matches.into_iter().take(a.match_limit).collect();
    let match_limit_reached = total_match_count > a.match_limit;
    if capped.is_empty() {
        return "No matches found".to_string();
    }

    let ws_str = a.workspace.to_string_lossy();
    let ws_prefix_slash = format!("{}/", ws_str);
    let cfg = BlockConfig {
        ws_str: ws_str.as_ref(),
        ws_prefix_slash: &ws_prefix_slash,
        context_lines: a.context_lines,
        max_line_bytes: a.max_line_bytes,
        max_output_bytes: a.max_output_bytes,
    };
    let mut file_cache: HashMap<PathBuf, Vec<String>> = HashMap::new();
    let mut state = FormatState {
        output_lines: Vec::new(),
        byte_total: 0,
        lines_truncated: false,
        truncated_bytes: false,
    };

    for m in &capped {
        // Validate each file path from rg JSON against the sandbox.
        // rg runs inside the validated search path, but a symlink inside the workspace
        // could resolve to a path outside it. sandbox.validate_path() catches this.
        let path_str = m.file_path.to_string_lossy();
        if a.sandbox.validate_path(&path_str).is_err() {
            // Skip files that violate the workspace boundary (symlink traversal etc.)
            continue;
        }
        if !format_match_block(m, &mut file_cache, &cfg, &mut state) {
            break;
        }
    }

    if state.output_lines.is_empty() {
        return "No matches found".to_string();
    }

    let mut output = state.output_lines.join("\n");
    let mut notices: Vec<String> = Vec::new();

    if match_limit_reached {
        notices.push(format!(
            "{} matches limit reached. Use limit={} for more, or refine pattern",
            a.match_limit,
            a.match_limit * 2
        ));
    }
    if state.truncated_bytes {
        notices.push(format!("{} limit reached", format_size(a.max_output_bytes)));
    }
    if state.lines_truncated {
        notices.push(format!(
            "Some lines truncated to {} chars. Use read tool to see full lines",
            a.max_line_bytes
        ));
    }
    if !notices.is_empty() {
        output.push_str(&format!("\n\n[{}]", notices.join(". ")));
    }
    output
}

// ---------------------------------------------------------------------------
// File cache and line helpers
// ---------------------------------------------------------------------------

/// Read a file into a line vector for the context cache.
/// Caps at `MAX_FILE_CACHE_BYTES` to prevent OOM from large files.
fn read_file_for_cache(path: &Path) -> Vec<String> {
    use std::io::Read;
    let Ok(f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut buf = Vec::with_capacity(MAX_FILE_CACHE_BYTES.min(64 * 1024));
    if f.take(MAX_FILE_CACHE_BYTES as u64)
        .read_to_end(&mut buf)
        .is_err()
    {
        return Vec::new();
    }
    String::from_utf8_lossy(&buf)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(str::to_string)
        .collect()
}

/// Truncate a line to max_bytes, appending a size hint if truncated.
/// Returns (display_text, was_truncated).
fn truncate_line(line: &str, max_bytes: usize) -> (String, bool) {
    if line.len() <= max_bytes {
        return (line.to_string(), false);
    }
    let end = (0..=max_bytes)
        .rev()
        .find(|&i| line.is_char_boundary(i))
        .unwrap_or(0);
    let size_hint = format_size(line.len());
    (format!("{}… [line is {}]", &line[..end], size_hint), true)
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
        let sandbox = Arc::new(Sandbox::new(Some(tmp.path().to_path_buf()), true));
        let tool = GrepTool::new(ws.clone(), sandbox);
        (tool, ws, tmp)
    }

    #[test]
    fn test_format_grep_output_empty() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(format_grep_output(GrepFormatArgs {
            json_output: "",
            sandbox: &Sandbox::new(None, false),
            workspace: &PathBuf::from("/ws"),
            match_limit: 100,
            context_lines: 0,
            max_line_bytes: 500,
            max_output_bytes: 50 * 1024,
        }));
        assert_eq!(result, "No matches found");
    }

    #[test]
    fn test_format_grep_output_basic() {
        // Single match event
        let json = r#"{"type":"match","data":{"path":{"text":"/ws/main.rs"},"line_number":1,"lines":{"text":"fn main() {}\n"},"absolute_offset":0,"submatches":[]}}"#;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ws = tempfile::TempDir::new().unwrap();
        std::fs::write(ws.path().join("main.rs"), "fn main() {}\n").unwrap();
        let json_patched = json.replace("/ws", &ws.path().to_string_lossy());
        let result = rt.block_on(format_grep_output(GrepFormatArgs {
            json_output: &json_patched,
            sandbox: &Sandbox::new(None, false),
            workspace: ws.path(),
            match_limit: 100,
            context_lines: 0,
            max_line_bytes: 500,
            max_output_bytes: 50 * 1024,
        }));
        assert!(result.contains("main.rs:1:"), "got: {}", result);
    }

    #[test]
    fn test_format_grep_output_match_limit() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ws = tempfile::TempDir::new().unwrap();
        let mut json_lines = Vec::new();
        for i in 1..=20usize {
            std::fs::write(
                ws.path().join(format!("f{}.rs", i)),
                format!("needle {}\n", i),
            )
            .unwrap();
            json_lines.push(format!(
                r#"{{"type":"match","data":{{"path":{{"text":"{}/f{}.rs"}},"line_number":1,"lines":{{"text":"needle {}\n"}},"absolute_offset":0,"submatches":[]}}}}"#,
                ws.path().to_string_lossy(), i, i
            ));
        }
        let json = json_lines.join("\n");
        let result = rt.block_on(format_grep_output(GrepFormatArgs {
            json_output: &json,
            sandbox: &Sandbox::new(None, false),
            workspace: ws.path(),
            match_limit: 5,
            context_lines: 0,
            max_line_bytes: 500,
            max_output_bytes: 50 * 1024,
        }));
        assert!(
            result.contains("5 matches limit reached"),
            "expected match limit notice, got: {}",
            &result[..result.len().min(200)]
        );
        assert!(
            result.contains("limit=10"),
            "expected limit=10, got: {}",
            result
        );
    }

    #[test]
    fn test_truncate_line_short() {
        let (result, was_truncated) = truncate_line("hello", 500);
        assert_eq!(result, "hello");
        assert!(!was_truncated);
    }

    #[test]
    fn test_truncate_line_long() {
        let long = "x".repeat(600);
        let (result, was_truncated) = truncate_line(&long, 500);
        assert!(result.contains("…"), "expected ellipsis");
        assert!(result.len() < 600);
        assert!(was_truncated);
    }

    #[test]
    fn test_parse_rg_matches() {
        let json = r#"{"type":"begin","data":{"path":{"text":"/ws/a.rs"}}}
{"type":"match","data":{"path":{"text":"/ws/a.rs"},"line_number":3,"lines":{"text":"target\n"},"absolute_offset":10,"submatches":[]}}
{"type":"end","data":{"path":{"text":"/ws/a.rs"},"stats":{}}}
{"type":"summary","data":{}}"#;
        let matches = parse_rg_matches(json);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].line_number, 3);
        assert_eq!(matches[0].file_path, PathBuf::from("/ws/a.rs"));
    }

    #[tokio::test]
    async fn test_grep_match_limit_notice_format() {
        let (tool, _ws, tmp) = test_grep();
        let content: String = (1..=50).map(|i| format!("needle {}\n", i)).collect();
        std::fs::write(tmp.path().join("many.txt"), content).unwrap();

        if std::process::Command::new("rg")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }

        let result = tool
            .execute(r#"{"pattern": "needle", "limit": 5}"#)
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
        assert!(
            result.content.contains("5 matches limit reached"),
            "expected match limit notice, got: {}",
            result.content
        );
        assert!(
            result.content.contains("limit=10"),
            "expected suggested limit, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_grep_context_uses_file_minus_format() {
        let (tool, _ws, tmp) = test_grep();
        std::fs::write(
            tmp.path().join("ctx.rs"),
            "line one\nfn target() {}\nline three\n",
        )
        .unwrap();

        if std::process::Command::new("rg")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }

        let result = tool
            .execute(r#"{"pattern": "target", "context": 1}"#)
            .await
            .unwrap();
        assert!(!result.is_error, "got: {}", result.content);
        assert!(
            result.content.contains("ctx.rs:2:"),
            "expected match line format, got: {}",
            result.content
        );
        assert!(
            result.content.contains("ctx.rs-1-") || result.content.contains("ctx.rs-3-"),
            "expected context line format (file-N-), got: {}",
            result.content
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

    // --- Fix 2: Actionable missing-parameter error ---

    #[tokio::test]
    async fn test_grep_empty_object_returns_actionable_error() {
        let (tool, _ws, _tmp) = test_grep();
        let result = tool.execute("{}").await.unwrap();
        assert!(result.is_error, "expected error, got: {}", result.content);
        assert!(
            result.content.contains("pattern"),
            "should mention missing 'pattern', got: {}",
            result.content
        );
        assert!(
            result.content.contains("Example"),
            "should include example, got: {}",
            result.content
        );
    }

    // --- Fix 3: Description includes example ---

    #[test]
    fn test_grep_description_includes_example() {
        let (tool, _ws, _tmp) = test_grep();
        let def = tool.definition();
        assert!(
            def.description.contains("Example"),
            "grep description should include Example, got: {}",
            def.description
        );
    }
}
