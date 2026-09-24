// Grep tool — ripgrep-powered file content search (Quecto compatibility).
// Uses `rg --json` for robust structured match extraction; files and count
// modes use rg's `--null` listings (#2136). Context lines are extracted from
// a file cache, not rg's --context output.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::tool::{ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::path_utils::resolve_to_cwd;
use crate::infrastructure::tools::truncate::format_size;

#[path = "grep_listing.rs"]
mod grep_listing;
#[path = "grep_request.rs"]
mod grep_request;
#[path = "grep_run.rs"]
mod grep_run;

use grep_listing::{ListingFormat, format_listing, parse_listing};
use grep_request::{DEFAULT_MATCH_LIMIT, GrepRequest, OutputMode, parse_request};
use grep_run::{RG_STDOUT_CAP, RG_TIMEOUT, run_rg};

/// Maximum line length before truncation (chars); matches Quecto's GREP_MAX_LINE_LENGTH.
const MAX_LINE_BYTES: usize = 500;
/// Maximum total output bytes (50KB); matches Quecto's DEFAULT_MAX_BYTES.
const MAX_OUTPUT_BYTES: usize = crate::domain::constants::DEFAULT_OUTPUT_CAP_BYTES;
/// Maximum individual file size for context reads (1MB); prevents OOM from huge cached files.
const MAX_FILE_CACHE_BYTES: usize = 1024 * 1024;
/// A match line past the file cache (#2136).
const BEYOND_CACHE: &str =
    "[line not shown: past the first 1 MB, or the file could not be read; read it directly]";

pub struct GrepTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
    /// Override the `rg` binary path (for testing with a dummy binary).
    rg_binary: Option<String>,
    /// How long one rg run may take.
    rg_timeout: std::time::Duration,
}

impl GrepTool {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self {
            workspace,
            sandbox,
            rg_binary: None,
            rg_timeout: RG_TIMEOUT,
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
            rg_timeout: RG_TIMEOUT,
        }
    }

    /// For tests: a shorter rg timeout.
    #[cfg(any(test, feature = "test-support"))]
    pub fn with_rg_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.rg_timeout = timeout;
        self
    }

    fn rg_cmd(&self) -> String {
        self.rg_binary.clone().unwrap_or_else(|| "rg".to_string())
    }
}

impl Tool for GrepTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "grep".into(),
            description: format!(
                "Search file contents with ripgrep. USE THIS TOOL FOR ALL CONTENT SEARCH: do not \
                                  run rg, grep or git grep through bash. It takes the rg options agents use most \
                 (several patterns, globs and file types, whole words, multiline, a per-file cap, \
                 files-only and match-count output), skips .git internals, keeps output bounded \
                 and checks paths against the sandbox. \
                 Returns file:line: content matches with optional context lines (file-N- format), \
                 capped at {} matches (or files) or {}KB. Use output=files or output=count to scope \
                 a search cheaply before reading matches. \
                 Example: {{\"pattern\": \"search_term\", \"type\": [\"rust\"]}}",
                DEFAULT_MATCH_LIMIT,
                MAX_OUTPUT_BYTES / 1024
            )
            .into(),
            parameters_schema: r#"{
                "type": "object",
                "properties": {
                                        "pattern":    {"type":"string","description":"Search pattern (regex, or literal with literal=true). Required unless patterns is given"},
                    "patterns":   {"type":"array","items":{"type":"string"},"description":"Several patterns; a line matching any of them matches (with or instead of pattern)"},
                    "path":       {"type":"string","description":"Directory or file to search (defaults to '.')"},
                    "glob":       {"type":"array","items":{"type":"string"},"description":"Globs to include, e.g. ['*.rs']; prefix with ! to exclude, e.g. '!*_tests.rs' (a single string is accepted too). .git internals are always skipped; search them with path='.git'"},
                    "type":       {"type":"array","items":{"type":"string"},"description":"ripgrep file types, e.g. ['rust'], ['py', 'ts'] (a single string is accepted too)"},
                    "ignoreCase": {"type":"boolean","description":"Case-insensitive search"},
                    "literal":    {"type":"boolean","description":"Treat patterns as literal strings"},
                    "wordRegexp": {"type":"boolean","description":"Match whole words only"},
                    "multiline":  {"type":"boolean","description":"Let patterns span lines (use \\n in the pattern)"},
                    "maxPerFile": {"type":"number","description":"At most this many matching lines per file"},
                    "output":     {"type":"string","enum":["content","files","count"],"description":"content (default): matching lines; files: each matching file once; count: number of matches per file, busiest first"},
                    "context":    {"type":"number","description":"Context lines before and after each match (content output)"},
                    "limit":      {"type":"number","description":"Maximum matches (or files) to return (default 100)"}
                }
            }"#
            .into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args: Result<serde_json::Value, _> = serde_json::from_str(arguments);
        let workspace = self.workspace.clone();
        let sandbox = self.sandbox.clone();
        let rg_cmd = self.rg_cmd();
        let rg_timeout = self.rg_timeout;

        Box::pin(async move {
            // LLM-addressable: malformed JSON → Ok(is_error=true). Tool contract.
            let args = match args {
                Ok(v) => v,
                Err(e) => {
                    return Ok(ToolResult {
                        content: format!(
                            "invalid JSON arguments: {e}. Example: {{\"pattern\": \"search_term\"}}"
                        ),
                        is_error: true,
                        image_blocks: vec![],
                        delivery_metadata: None,
                    });
                }
            };

            let request = match parse_request(&args) {
                Ok(request) => request,
                Err(problem) => {
                    return Ok(ToolResult {
                        content: problem,
                        is_error: true,
                        image_blocks: vec![],
                        delivery_metadata: None,
                    });
                }
            };

            let full_path = resolve_to_cwd(&request.path, &workspace);
            let full_str = full_path.to_string_lossy().to_string();
            sandbox
                .validate_path(&full_str)
                .map_err(|e| DomainError::Security(e.to_string()))?;

            let cmd = build_rg_command(&rg_cmd, &workspace, &full_path, &request);
            let rg = run_rg(cmd, rg_timeout).await?;
            let stderr = String::from_utf8_lossy(&rg.stderr).into_owned();
            let stdout = String::from_utf8_lossy(&rg.stdout).into_owned();
            let found = match request.output {
                OutputMode::Content => parse_rg_matches(&stdout).len(),
                OutputMode::Files | OutputMode::Count => {
                    parse_listing(&stdout, request.output).len()
                }
            };
            // rg exits 0 (matches) or 1 (none). Exit 2 is an error, which may
            // follow partial results (an unreadable file) or nothing but a
            // summary (a mistyped path); no code means killed, here at the
            // output cap. Results stand when rg finished, or found something.
            // Capped before one whole match was read: its line alone is
            // larger than the cap (a minified file) — say so, not an error.
            if rg.capped && found == 0 {
                return Ok(ToolResult {
                    content: format!(
                        "A matching line is larger than {}, so no match could be shown: \
                         narrow the search with glob or type, or read the file directly",
                        format_size(RG_STDOUT_CAP)
                    ),
                    is_error: false,
                    image_blocks: vec![],
                    delivery_metadata: None,
                });
            }
            let usable = matches!(rg.exit_code, Some(0 | 1)) || found > 0;
            let Some(stdout) = usable.then_some(stdout) else {
                let msg = match (stderr.trim(), rg.exit_code) {
                    ("", Some(code)) => format!("rg failed with exit status {code} and no message"),
                    ("", None) => "rg exited unexpectedly".to_string(),
                    (reported, _) => format!("grep error: {reported}"),
                };
                return Ok(ToolResult {
                    content: msg,
                    is_error: true,
                    image_blocks: vec![],
                    delivery_metadata: None,
                });
            };
            let mut incomplete = Vec::new();
            // At the cap more exists than was read; say so unless the match
            // limit already cut the result shorter (its own notice says so).
            if rg.capped && found <= request.limit {
                incomplete.push(format!(
                    "rg printed more than {}; results are incomplete: narrow with path, glob or type",
                    format_size(RG_STDOUT_CAP)
                ));
            }
            // Stopped by a signal other than the tool's own at the cap.
            if rg.held_open {
                incomplete.push(
                    "rg's output was still held open after it exited; results may be incomplete"
                        .to_string(),
                );
            }
            if let (None, false) = (rg.exit_code, rg.capped) {
                incomplete.push("rg was stopped by a signal; results are incomplete".to_string());
            }
            if rg.exit_code == Some(2) {
                let first = stderr.lines().next().unwrap_or("").trim();
                incomplete.push(format!(
                    "rg reported errors, results may be incomplete: {first}"
                ));
            }
            let result = match request.output {
                OutputMode::Content => {
                    format_grep_output(GrepFormatArgs {
                        json_output: &stdout,
                        workspace: &workspace,
                        sandbox: &sandbox,
                        match_limit: request.limit,
                        context_lines: request.context_lines,
                        max_line_bytes: MAX_LINE_BYTES,
                        max_output_bytes: MAX_OUTPUT_BYTES,
                    })
                    .await
                }
                OutputMode::Files | OutputMode::Count => format_listing(
                    parse_listing(&stdout, request.output),
                    &ListingFormat {
                        workspace: &workspace,
                        sandbox: &sandbox,
                        limit: request.limit,
                        max_output_bytes: MAX_OUTPUT_BYTES,
                        total_is_partial: rg.capped,
                    },
                ),
            };

            let result = match incomplete.as_slice() {
                [] => result,
                notices => format!("{result}\n\n[{}]", notices.join(". ")),
            };
            Ok(ToolResult {
                content: result,
                is_error: false,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

// ---------------------------------------------------------------------------
// rg invocation
// ---------------------------------------------------------------------------

/// Build the ripgrep command for `request`: `--json` for matching lines
/// (context comes from a file cache, not `--context`), `--null` listings
/// for files and counts. Patterns always go through `-e` and the path
/// after `--`, so neither can be read as a flag.
fn build_rg_command(
    rg_cmd: &str,
    workspace: &Path,
    full_path: &Path,
    request: &GrepRequest,
) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(rg_cmd);
    cmd.current_dir(workspace)
        // A user's rg config could add --pre (a child process per file) or
        // change output: the tool's search is defined by its own flags.
        .arg("--no-config")
        .arg("--line-number")
        .arg("--color=never")
        .arg("--hidden");
    match request.output {
        OutputMode::Content => cmd.arg("--json"),
        OutputMode::Files => cmd.arg("--files-with-matches").arg("--null"),
        OutputMode::Count => cmd
            .arg("--count-matches")
            .arg("--with-filename")
            .arg("--null"),
    };
    // --hidden searches dotfiles, but a repository's .git internals are
    // never what a content search means (bash rg skips them too).
    cmd.arg("--glob").arg("!.git");
    let flags = [
        (request.ignore_case, "--ignore-case"),
        (request.literal, "--fixed-strings"),
        (request.word, "--word-regexp"),
        (request.multiline, "--multiline"),
    ];
    for (on, flag) in flags {
        if on {
            cmd.arg(flag);
        }
    }
    if let Some(max) = request.max_per_file {
        cmd.arg("--max-count").arg(max.to_string());
    }
    for glob in &request.globs {
        cmd.arg("--glob").arg(glob);
    }
    for file_type in &request.types {
        cmd.arg("--type").arg(file_type);
    }
    for pattern in &request.patterns {
        cmd.arg("-e").arg(pattern);
    }
    cmd.arg("--").arg(full_path);
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    cmd
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
    /// Lines the match spans (more than one only for a multiline pattern).
    line_count: usize,
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
        // rg reports the lines a match spans in `lines.text`, ending in a
        // newline: a multiline match spans several (#2136).
        let line_count = spanned_lines(&event["data"]["lines"]);
        matches.push(RgMatch {
            file_path: PathBuf::from(file_path),
            line_number: line_number as usize,
            line_count,
        });
    }
    matches
}

/// How many lines a match spans: rg gives them as `text`, or as base64
/// `bytes` when they are not valid UTF-8.
fn spanned_lines(lines: &serde_json::Value) -> usize {
    use base64::Engine as _;
    let newlines = match (lines["text"].as_str(), lines["bytes"].as_str()) {
        (Some(text), _) => text.trim_end_matches('\n').matches('\n').count(),
        (None, Some(encoded)) => base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_or(0, |bytes| {
                let trimmed = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
                trimmed.iter().filter(|b| **b == b'\n').count()
            }),
        (None, None) => 0,
    };
    newlines + 1
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

/// Format parsed matches with file-cache-based context extraction (Quecto compatibility).
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
    // A search of "." reports `<workspace>/./x`: show `x`, as files mode does.
    let rel_path = rel_path.strip_prefix("./").unwrap_or(rel_path);

    let total_lines = file_lines.len();
    let last_matched = m.line_number + m.line_count.max(1) - 1;
    let start = m.line_number.saturating_sub(cfg.context_lines).max(1);
    let end = (last_matched + cfg.context_lines)
        .min(total_lines.max(last_matched))
        .max(m.line_number);

    for current in start..=end {
        let matched = (m.line_number..=last_matched).contains(&current);
        let line_text = match (file_lines.get(current - 1), matched) {
            (Some(line), _) => line.as_str(),
            // Past what the context cache read (the first 1 MB): say so
            // rather than print an empty match line; skip empty context.
            (None, true) => BEYOND_CACHE,
            (None, false) => continue,
        };
        let sanitized = line_text.trim_end_matches('\n');
        let (display_text, was_truncated) = truncate_line(sanitized, cfg.max_line_bytes);
        if was_truncated {
            state.lines_truncated = true;
        }

        let formatted = if matched {
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
    // Pre-populate file cache via spawn_blocking to avoid blocking the Tokio
    // runtime thread. Each file read can be up to MAX_FILE_CACHE_BYTES (1MB).
    let unique_paths: Vec<PathBuf> = {
        let mut seen = std::collections::HashSet::new();
        capped
            .iter()
            .filter(|m| {
                a.sandbox
                    .validate_path(&m.file_path.to_string_lossy())
                    .is_ok()
            })
            .filter(|m| seen.insert(m.file_path.clone()))
            .map(|m| m.file_path.clone())
            .collect()
    };
    let mut file_cache: HashMap<PathBuf, Vec<String>> = HashMap::new();
    for path in unique_paths {
        let p = path.clone();
        let lines = tokio::task::spawn_blocking(move || read_file_for_cache(&p))
            .await
            .unwrap_or_default();
        file_cache.insert(path, lines);
    }

    let mut state = FormatState {
        output_lines: Vec::new(),
        byte_total: 0,
        lines_truncated: false,
        truncated_bytes: false,
    };

    for m in &capped {
        // Route each rg-reported file path through the shared path hook before
        // reading context. Agent entrypoints no longer enable workspace
        // confinement, but keeping this call preserves one policy chokepoint.
        let path_str = m.file_path.to_string_lossy();
        if a.sandbox.validate_path(&path_str).is_err() {
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
#[path = "grep_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "grep_cov_tests.rs"]
mod cov_tests;

#[cfg(test)]
mod install_guidance_tests {
    use super::*;

    #[tokio::test]
    async fn absent_rg_reports_actionable_install_guidance() {
        let workspace = tempfile::tempdir().expect("workspace");
        let missing = workspace.path().join("absent-rg");
        assert!(!missing.exists(), "fixture executable must be absent");
        let tool = GrepTool::with_rg_binary(
            Arc::new(workspace.path().to_path_buf()),
            Arc::new(Sandbox::new(Some(workspace.path().to_path_buf()))),
            missing.to_string_lossy().into_owned(),
        );
        let result = tool.execute(r#"{"pattern":"needle"}"#).await;
        let message = match result {
            Ok(output) if output.is_error => output.content,
            Err(error) => error.to_string(),
            other => panic!("expected missing executable error, got {other:?}"),
        };
        assert!(message.contains("rg not found on PATH"), "{message}");
        assert!(
            message.contains("https://github.com/BurntSushi/ripgrep#installation"),
            "{message}"
        );
    }
}
