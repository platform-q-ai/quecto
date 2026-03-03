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

/// Returns `true` when a glob pattern contains a `/`, meaning it encodes a
/// directory path segment (e.g. `src/*.rs`, `nested/*.txt`, `a/b/c.json`).
///
/// fd's `--glob` mode by default matches only against the **filename** (last
/// path component). To match path-segment patterns we must add `--full-path`
/// to the fd invocation and ensure the pattern starts with `**/` so it is
/// anchored against any prefix.
pub(crate) fn pattern_has_path_segment(pattern: &str) -> bool {
    pattern.contains('/')
}

/// Prepare a glob pattern for use with `fd --glob --full-path`.
///
/// Normalises and anchors path-segment patterns so fd matches them correctly
/// against absolute paths:
///
/// 1. Strip a leading `./` — fd's glob engine does not normalise `./` in
///    full-path mode, so `**/./src/*.rs` silently matches nothing.
/// 2. Strip a leading `/` — an absolute-looking pattern (e.g. `/src/*.rs`)
///    would never match a file whose full path starts with the workspace root.
/// 3. Prepend `**/` to non-anchored patterns so they match at any depth.
///    Patterns already starting with `**` are returned unchanged.
///
/// Patterns without a `/` (e.g. `*.rs`) are returned unchanged because
/// full-path mode is not used for them.
///
/// Examples:
/// - `"src/*.rs"`    → `"**/src/*.rs"`   (path-segment, not anchored)
/// - `"./src/*.rs"`  → `"**/src/*.rs"`   (leading ./ stripped)
/// - `"/src/*.rs"`   → `"**/src/*.rs"`   (leading / stripped)
/// - `"**/*.rs"`     → `"**/*.rs"`       (already anchored with **)
/// - `"*.rs"`        → `"*.rs"`          (no slash — full-path mode unused)
pub(crate) fn build_full_path_pattern(pattern: &str) -> String {
    if !pattern_has_path_segment(pattern) {
        // No slash — full-path mode is not used; return unchanged.
        return pattern.to_string();
    }
    // Normalise: strip leading ./ or / so the pattern can be anchored with **/.
    let pat = pattern
        .strip_prefix("./")
        .or_else(|| pattern.strip_prefix('/'))
        .unwrap_or(pattern);
    if pat.starts_with("**") {
        // Already anchored — return the normalised form.
        return pat.to_string();
    }
    format!("**/{}", pat)
}

fn missing_pattern_error() -> ToolResult {
    ToolResult {
        content: "missing 'pattern' argument. Example: {\"pattern\": \"*.rs\"}".to_string(),
        is_error: true,
        image_blocks: vec![],
    }
}

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
            name: "find".into(),
            description: "Find files by glob pattern using fd. Requires fd on PATH. \
                          Returns newline-separated relative paths. Respects .gitignore. \
                          Output capped at 1000 results or 50KB. \
                          Example: {\"pattern\": \"*.rs\"}"
                .into(),
            parameters_schema: r#"{
                "type": "object",
                "properties": {
                    "pattern": {"type":"string","description":"Glob pattern, e.g. '*.rs', '**/*.json', or 'src/*.rs' (path-segment patterns work)"},
                    "path":    {"type":"string","description":"Directory to search (defaults to '.')"},
                    "limit":   {"type":"number","description":"Maximum results (default 1000)"}
                },
                "required": ["pattern"]
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
        let fd_cmd = self.fd_cmd();

        Box::pin(async move {
            let args = args.map_err(|e| DomainError::Tool(e.to_string()))?;

            let Some(pattern) = args["pattern"].as_str() else {
                return Ok(missing_pattern_error());
            };

            let search_path = args["path"].as_str().unwrap_or(".");
            let full_path = resolve_to_cwd(search_path, &workspace);
            sandbox
                .validate_path(full_path.to_string_lossy().as_ref())
                .map_err(|e| DomainError::Security(e.to_string()))?;

            // Accept float limits (JSON "number" type); cap at 100_000 to prevent
            // pathological fd invocations.
            const MAX_RESULT_LIMIT: usize = 100_000;
            let limit = args["limit"]
                .as_f64()
                .map(|v| (v.round() as usize).clamp(1, MAX_RESULT_LIMIT))
                .unwrap_or(DEFAULT_RESULT_LIMIT);

            let gitignore_files = discover_gitignore_files(&full_path);
            let (stdout_bytes, stderr_bytes, exit_code) = run_fd(FdArgs {
                fd_cmd: &fd_cmd,
                workspace: &workspace,
                pattern,
                search_dir: &full_path,
                limit,
                gitignore_files: &gitignore_files,
            })
            .await?;

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
                    image_blocks: vec![],
                });
            }

            let result = format_find_output(&stdout, &full_path, limit, MAX_OUTPUT_BYTES);
            Ok(ToolResult {
                content: result,
                is_error: false,
                image_blocks: vec![],
            })
        })
    }
}

// ---------------------------------------------------------------------------
// fd invocation
// ---------------------------------------------------------------------------

struct FdArgs<'a> {
    fd_cmd: &'a str,
    workspace: &'a std::path::Path,
    pattern: &'a str,
    /// The resolved, sandbox-validated directory to search.
    search_dir: &'a std::path::Path,
    limit: usize,
    gitignore_files: &'a [PathBuf],
}

/// Spawn fd, read capped stdout/stderr, reap child.
///
/// When the pattern contains a path separator (e.g. `"src/*.rs"`), fd's
/// default `--glob` mode only tests the pattern against the filename component
/// and would silently return nothing. We add `--full-path` and use
/// `build_full_path_pattern` to anchor the pattern so it matches at any depth.
async fn run_fd(a: FdArgs<'_>) -> Result<(Vec<u8>, Vec<u8>, Option<i32>), DomainError> {
    use std::borrow::Cow;
    use tokio::io::AsyncReadExt;

    let needs_full_path = pattern_has_path_segment(a.pattern);
    // Avoid heap allocation in the common case (no path segment).
    let effective_pattern: Cow<'_, str> = if needs_full_path {
        Cow::Owned(build_full_path_pattern(a.pattern))
    } else {
        Cow::Borrowed(a.pattern)
    };

    // Build: fd --glob [--full-path] --color=never --hidden --max-results N
    //           [--ignore-file ...] -- <pattern> <search_dir>
    let mut cmd = tokio::process::Command::new(a.fd_cmd);
    cmd.current_dir(a.workspace)
        .arg("--glob")
        .arg("--color=never")
        .arg("--hidden")
        .arg("--max-results")
        .arg(a.limit.to_string());
    if needs_full_path {
        cmd.arg("--full-path");
    }
    for gitignore in a.gitignore_files {
        cmd.arg("--ignore-file").arg(gitignore);
    }
    cmd.arg("--")
        .arg(effective_pattern.as_ref())
        .arg(a.search_dir)
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

    let mut stderr_bytes = Vec::new();
    if let Some(mut err) = child.stderr.take() {
        // Cap stderr at 4 KiB — enough for any error message, prevents OOM.
        let mut tmp = vec![0u8; 4096];
        let n = err.read(&mut tmp).await.unwrap_or(0);
        stderr_bytes.extend_from_slice(&tmp[..n]);
    }

    let _ = child.kill().await;
    let status = child.wait().await;
    let exit_code = status.ok().and_then(|s| s.code());
    Ok((stdout_bytes, stderr_bytes, exit_code))
}

/// Discover the `.gitignore` file at the root of `search_dir`.
///
/// fd natively respects `.gitignore` files inside git repositories. For
/// non-git-repo trees, we pass the **root-level** `.gitignore` via
/// `--ignore-file` so its rules are applied.
///
/// **Only the root `.gitignore` is returned.** Nested `.gitignore` files are
/// NOT passed via `--ignore-file` because fd applies `--ignore-file` rules
/// **globally** — a `*.json` rule in `vendor/.gitignore` would suppress
/// `.json` files everywhere, not just under `vendor/`. Within a git repo,
/// fd already handles nested `.gitignore` scoping correctly via git's
/// native ignore machinery.
///
/// **Tradeoff**: In non-git workspaces, nested `.gitignore` files are no
/// longer respected. This is acceptable because (a) the global application
/// bug caused incorrect results (files missing from find output), and
/// (b) non-git workspaces with meaningful nested `.gitignore` are rare.
///
/// **Catch-all filtering** still applies: a root `.gitignore` whose only
/// rules are `*`, `**`, `**/`, or `**/*` (plus negations/comments) is
/// excluded to prevent blanket suppression.
///
/// Safety:
/// - Only reads the search root directory (no recursive traversal)
/// - Uses `symlink_metadata` to reject symlinks (prevents traversal outside workspace)
pub(crate) fn discover_gitignore_files(search_dir: &std::path::Path) -> Vec<PathBuf> {
    let gitignore_path = search_dir.join(".gitignore");
    // Use symlink_metadata so a symlink to a .gitignore outside the workspace
    // is rejected (is_file() returns false for symlinks).
    match std::fs::symlink_metadata(&gitignore_path) {
        Ok(meta) if meta.is_file() && !is_catch_all_gitignore(&gitignore_path) => {
            vec![gitignore_path]
        }
        _ => Vec::new(),
    }
}

/// Maximum bytes read from a single `.gitignore` file when checking for catch-all rules.
/// Guards against OOM from a malformed or adversarially crafted file with no newlines.
const MAX_GITIGNORE_READ_BYTES: u64 = 64 * 1024;

/// Returns `true` when a `.gitignore` file contains a catch-all rule that
/// would suppress every file if applied globally via `--ignore-file`.
///
/// Catch-all patterns detected: `*`, `**`, `**/`, `**/*`.
/// A gitignore is considered catch-all when it contains only such patterns
/// plus blank lines, comments, and negations — no real content rules.
///
/// Examples that are catch-all (excluded):
/// - `*`
/// - `*\n!.gitignore`
/// - `**\n# comment\n!README.md`
///
/// Examples that are NOT catch-all (included):
/// - `target/\n*.log`
/// - `node_modules/`
pub(crate) fn is_catch_all_gitignore(path: &std::path::Path) -> bool {
    use std::io::{BufRead, BufReader, Read};
    // Size guard: skip unreasonably large files rather than risk OOM.
    // A legitimate .gitignore is never 64 KiB.
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() > MAX_GITIGNORE_READ_BYTES {
            return false;
        }
    }
    let Ok(f) = std::fs::File::open(path) else {
        return false;
    };
    // Wrap in a byte-capped reader as a second safety net.
    let capped = f.take(MAX_GITIGNORE_READ_BYTES);
    let mut has_catch_all = false;
    for line in BufReader::new(capped).lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue; // blank or comment — skip
        }
        if matches!(trimmed, "*" | "**" | "**/" | "**/*") {
            has_catch_all = true;
        } else if !trimmed.starts_with('!') {
            // A real content rule (not a negation) — cannot be a pure catch-all.
            return false;
        }
    }
    has_catch_all
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
#[path = "find_tests.rs"]
mod tests;
