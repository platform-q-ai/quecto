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
use crate::infrastructure::tools::truncate::format_size;

#[path = "grep_blocks.rs"]
mod grep_blocks;
#[path = "grep_listing.rs"]
mod grep_listing;
#[path = "grep_rank.rs"]
mod grep_rank;
#[path = "grep_request.rs"]
mod grep_request;
#[path = "grep_run.rs"]
mod grep_run;
#[path = "grep_search.rs"]
mod grep_search;

use grep_blocks::{BlockConfig, FormatState, format_match_block, matched_lines};
use grep_rank::Ranking;
use grep_request::{DEFAULT_MATCH_LIMIT, GrepRequest, OutputMode};
use grep_run::RG_TIMEOUT;
#[cfg(test)]
use grep_run::run_rg;
use grep_search::{PendingRecord, SearchContext, SearchFacts, search};

use crate::application::search::ports::{RelevanceJudge, SearchLog};

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
    /// `rank_by` relevance ranking, when configured (#2136 slice B).
    ranking: Option<Arc<Ranking>>,
    /// Where every search is recorded, when configured.
    search_log: Option<Arc<dyn SearchLog>>,
    /// Directories never searched (the search log's own).
    excluded: Vec<PathBuf>,
}

impl GrepTool {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self {
            workspace,
            sandbox,
            rg_binary: None,
            rg_timeout: RG_TIMEOUT,
            ranking: None,
            search_log: None,
            excluded: Vec::new(),
        }
    }

    /// Rank matches by relevance to `rank_by` through `judge`, judging at
    /// most `max_candidates` per search.
    pub fn with_relevance(mut self, judge: Arc<dyn RelevanceJudge>, max_candidates: usize) -> Self {
        assert!(max_candidates >= 1, "ranking judges at least one match");
        self.ranking = Some(Arc::new(Ranking {
            judge,
            max_candidates,
        }));
        self
    }

    /// Never show results from under `dir` (an absolute path; the search
    /// log's own).
    pub fn excluding(mut self, dir: PathBuf) -> Self {
        assert!(
            dir.is_absolute(),
            "an excluded directory is absolute: {}",
            dir.display()
        );
        self.excluded.push(dir);
        self
    }

    /// Record every search in `log`.
    pub fn with_search_log(mut self, log: Arc<dyn SearchLog>) -> Self {
        self.search_log = Some(log);
        self
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
            ranking: None,
            search_log: None,
            excluded: Vec::new(),
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
        let definition = base_definition();
        match &self.ranking {
            Some(_) => with_rank_by(definition),
            None => definition,
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let raw = arguments.to_string();
        let ctx = SearchContext {
            workspace: self.workspace.clone(),
            sandbox: self.sandbox.clone(),
            rg_cmd: self.rg_cmd(),
            rg_timeout: self.rg_timeout,
            ranking: self.ranking.clone(),
            excluded: self.excluded.clone(),
        };
        // Recorded once however the call ends: made before the future so a
        // call dropped before its first poll is recorded too.
        let pending = PendingRecord::new(self.search_log.clone(), &raw);

        Box::pin(async move {
            let mut facts = SearchFacts::default();
            let outcome = match serde_json::from_str::<serde_json::Value>(&raw) {
                Ok(args) => search(&ctx, &args, &mut facts).await,
                // LLM-addressable: malformed JSON → Ok(is_error=true). Tool contract.
                Err(e) => Ok(ToolResult {
                    content: format!(
                        "invalid JSON arguments: {e}. Example: {{\"pattern\": \"search_term\"}}"
                    ),
                    is_error: true,
                    image_blocks: vec![],
                    delivery_metadata: None,
                }),
            };
            pending.finish(facts, &outcome);
            outcome
        })
    }
}

/// The grep tool's definition without `rank_by`.
fn base_definition() -> ToolDefinition {
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

/// How the description introduces `rank_by`, when ranking is configured.
const RANK_BY_SENTENCE: &str = " When your words may match more than you mean, add rank_by: what you are \
     looking for, in words (e.g. \"where the retry delay is computed\"). Matches are then judged for \
     relevance and returned best first, each with its score; if ranking is unavailable you get the \
     plain results and a note.";

/// The definition with `rank_by` offered.
fn with_rank_by(mut definition: ToolDefinition) -> ToolDefinition {
    definition.description = format!("{}{RANK_BY_SENTENCE}", definition.description).into();
    let mut schema: serde_json::Value =
        serde_json::from_str(&definition.parameters_schema).expect("the grep schema is JSON");
    schema["properties"]["rank_by"] = serde_json::json!({
        "type": "string",
        "description": "What you are looking for, in words: matches are judged for relevance and returned best first (content output only)"
    });
    definition.parameters_schema = schema.to_string().into();
    definition
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
    /// Its relevance to `rank_by`, when judged (#2136 slice B).
    score: Option<f64>,
    /// Where the match starts in its first line (a byte offset), as rg
    /// reports it: a long line is shown to the judge around this.
    column: Option<usize>,
    /// The lines the match spans, as rg reported them (#2163): shown for the
    /// match however far into its file it is, where the file cache holds
    /// only the first 1 MB (and is used for context).
    text: Option<Vec<String>>,
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
        let text = reported_lines(&event["data"]["lines"]);
        matches.push(RgMatch {
            file_path: PathBuf::from(file_path),
            line_number: line_number as usize,
            line_count,
            score: None,
            column: event["data"]["submatches"][0]["start"]
                .as_u64()
                .and_then(|start| usize::try_from(start).ok()),
            text,
        });
    }
    matches
}

/// The lines rg reports a match spanning, without their line breaks: from
/// `text`, or decoded (lossily) from base64 `bytes` when not valid UTF-8.
fn reported_lines(lines: &serde_json::Value) -> Option<Vec<String>> {
    use base64::Engine as _;
    let text = match (lines["text"].as_str(), lines["bytes"].as_str()) {
        (Some(text), _) => text.to_string(),
        (None, Some(encoded)) => {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .ok()?;
            String::from_utf8_lossy(&bytes).into_owned()
        }
        (None, None) => return None,
    };
    let text = text.strip_suffix('\n').unwrap_or(&text);
    Some(
        text.split('\n')
            .map(|line| line.trim_end_matches('\r').to_string())
            .collect(),
    )
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

/// How matches are formatted: bounds and where paths are relative to.
struct MatchFormat<'a> {
    workspace: &'a Path,
    sandbox: &'a Sandbox,
    match_limit: usize,
    context_lines: usize,
    max_line_bytes: usize,
    max_output_bytes: usize,
}

/// Tests format raw `rg --json` output directly.
#[cfg(test)]
struct GrepFormatArgs<'a> {
    json_output: &'a str,
    workspace: &'a Path,
    sandbox: &'a Sandbox,
    match_limit: usize,
    context_lines: usize,
    max_line_bytes: usize,
    max_output_bytes: usize,
}

#[cfg(test)]
async fn format_grep_output(a: GrepFormatArgs<'_>) -> String {
    let format = MatchFormat {
        workspace: a.workspace,
        sandbox: a.sandbox,
        match_limit: a.match_limit,
        context_lines: a.context_lines,
        max_line_bytes: a.max_line_bytes,
        max_output_bytes: a.max_output_bytes,
    };
    format_matches(parse_rg_matches(a.json_output), &format).await
}

/// Format matches in the order given (rg's, or relevance order).
async fn format_matches(all_matches: Vec<RgMatch>, a: &MatchFormat<'_>) -> String {
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
    let matched = matched_lines(&capped);
    let cfg = BlockConfig {
        ws_str: ws_str.as_ref(),
        ws_prefix_slash: &ws_prefix_slash,
        context_lines: a.context_lines,
        max_line_bytes: a.max_line_bytes,
        max_output_bytes: a.max_output_bytes,
        matched: &matched,
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
        printed_through: None,
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
    read_capped_lines(path).0
}

/// A file's lines up to `MAX_FILE_CACHE_BYTES`, and whether the last of
/// them was cut part-way by that cap: decided from the bytes read (one past
/// the cap), never from a second look at a file that may have changed.
fn read_capped_lines(path: &Path) -> (Vec<String>, bool) {
    use std::io::Read;
    let Ok(f) = std::fs::File::open(path) else {
        return (Vec::new(), false);
    };
    let mut buf = Vec::with_capacity(MAX_FILE_CACHE_BYTES.min(64 * 1024));
    if f.take(MAX_FILE_CACHE_BYTES as u64 + 1)
        .read_to_end(&mut buf)
        .is_err()
    {
        return (Vec::new(), false);
    }
    // Past the cap, the last line kept is whole only when a line break ends
    // it: at the cap's last byte, or just after it.
    let cut = match buf.get(MAX_FILE_CACHE_BYTES) {
        Some(b'\n' | b'\r') => false,
        Some(_) => buf[MAX_FILE_CACHE_BYTES - 1] != b'\n',
        None => false,
    };
    buf.truncate(MAX_FILE_CACHE_BYTES);
    let lines = String::from_utf8_lossy(&buf)
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .map(str::to_string)
        .collect();
    (lines, cut)
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
#[path = "grep_rank_by_tests.rs"]
mod rank_by_tests;

#[cfg(test)]
#[path = "grep_read_limit_tests.rs"]
mod read_limit_tests;

#[cfg(test)]
#[path = "grep_output_tests.rs"]
mod output_tests;

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
