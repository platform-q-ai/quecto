// Filesystem tools: read, write, edit, append_file, list_dir.

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

            // Safety cap: reject reads > 10 MiB before loading into memory.
            // Truncation handles presentation limits; this prevents OOM on huge files.
            const MAX_READ_BYTES: u64 = 10 * 1024 * 1024;
            if let Ok(meta) = tokio::fs::metadata(&resolved).await {
                if meta.len() > MAX_READ_BYTES {
                    let size = format_size(meta.len() as usize);
                    let hint = shell_escape_single(path);
                    return Ok(ToolResult {
                        content: format!(
                            "File is {size} — too large to read directly (max 10 MiB). \
                             Use bash: head -n 2000 {hint} | head -c 51200",
                        ),
                        is_error: true,
                    });
                }
            }

            // Load file content
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

/// Wrap a path in single quotes for use in a shell command hint.
/// Escapes any embedded single quotes using the standard `'\''` trick.
fn shell_escape_single(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

/// Apply offset/limit pagination and truncation to file content.
/// Returns the formatted output string with optional continuation hints.
///
/// # Offset semantics
/// - `None` → start from line 1
/// - `Some(0)` → **error** (1-indexed; 0 is not a valid line number)
/// - `Some(n)` → start from line n (1-indexed)
fn apply_read_truncation(
    content: &str,
    path: &str,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<String, DomainError> {
    // Validate offset before counting lines — zero is never valid for 1-indexed API
    if offset == Some(0) {
        return Err(DomainError::Tool(
            "offset is 1-indexed; 0 is not valid. Use offset=1 for the first line.".to_string(),
        ));
    }

    let total_lines: usize = content.lines().count();

    // Convert 1-indexed offset to 0-indexed skip count
    let start_line = match offset {
        None => 0,
        Some(n) => {
            if n > total_lines {
                return Err(DomainError::Tool(format!(
                    "Offset {} is beyond end of file ({} lines total)",
                    n, total_lines
                )));
            }
            n - 1
        }
    };

    // Determine effective max_lines
    let max_lines = limit.unwrap_or(DEFAULT_MAX_LINES);

    // Build sliced content using iterator — avoids allocating an intermediate Vec
    let sliced: String = {
        let mut lines = content.lines().skip(start_line);
        let mut buf = String::new();
        let mut first = true;
        for ln in lines.by_ref() {
            if !first {
                buf.push('\n');
            }
            buf.push_str(ln);
            first = false;
        }
        buf
    };

    // Apply head-truncation
    let tr = truncate_head(&sliced, max_lines, DEFAULT_MAX_BYTES);

    // Build output with hints
    let mut output = String::new();

    if tr.first_line_exceeds_limit {
        // Single line exceeds byte limit — suggest a shell command
        let line_size = format_size(sliced.lines().next().map_or(0, str::len));
        let limit_size = format_size(DEFAULT_MAX_BYTES);
        let escaped = shell_escape_single(path);
        output.push_str(&format!(
            "[Line {} is {}, exceeds {} limit. Use bash: sed -n '{}p' {escaped} | head -c {}]",
            start_line + 1,
            line_size,
            limit_size,
            start_line + 1,
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
// EditTool  (Pi name: "edit")
// ===========================================================================

pub struct EditTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
}

impl EditTool {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self { workspace, sandbox }
    }
}

impl Tool for EditTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "edit".to_string(),
            description: "Edit a file by replacing exact text. The oldText must match exactly \
                          (including whitespace). Use this for precise, surgical edits."
                .to_string(),
            parameters_schema: r#"{"type":"object","properties":{
                "path":{"type":"string","description":"Path to the file to edit (relative or absolute)"},
                "oldText":{"type":"string","description":"Exact text to find and replace (must match exactly)"},
                "newText":{"type":"string","description":"New text to replace the old text with"}
            },"required":["path","oldText","newText"]}"#.to_string(),
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
            // Accept both "oldText" (Pi name) and legacy "old"
            let old_text = args["oldText"]
                .as_str()
                .or_else(|| args["old"].as_str())
                .ok_or_else(|| DomainError::Tool("missing 'oldText' argument".to_string()))?;
            // Accept both "newText" (Pi name) and legacy "new"
            let new_text = args["newText"]
                .as_str()
                .or_else(|| args["new"].as_str())
                .ok_or_else(|| DomainError::Tool("missing 'newText' argument".to_string()))?;

            let full_path = resolve_and_validate(&workspace, &sandbox, path)?;
            enforce_edit_file_size_limit(&full_path).await?;

            let raw = tokio::fs::read_to_string(&full_path)
                .await
                .map_err(|e| DomainError::Tool(format!("edit read failed: {}", e)))?;

            // Normalise only for *matching purposes* — we write back to raw to preserve
            // the file's original line endings outside the edited region.
            let content = fuzzy_normalise(&raw);
            let normalised_old = fuzzy_normalise(old_text);

            // Single-pass: count occurrences (capped at 2) and get first match offset.
            let (count, match_offset) = count_occurrences_capped(&content, &normalised_old, 2);
            if count == 0 {
                return Ok(ToolResult {
                    content: format!("oldText not found in {}", path),
                    is_error: true,
                });
            }
            if count > 1 {
                return Ok(ToolResult {
                    content: format!(
                        "oldText matches {} times in {} — it must match exactly once to avoid \
                         ambiguous edits. Add more context to make it unique.",
                        count, path
                    ),
                    is_error: true,
                });
            }

            let offset = match_offset.expect("count=1 guarantees Some(offset)");
            let normalised_new = fuzzy_normalise(new_text);

            // Write-back: apply replacement to the normalised content (already allocated)
            // rather than trying to map back into raw bytes (complex and error-prone).
            // The file's content becomes LF-only — this is a deliberate trade-off:
            // fuzzy matching of CRLF files must normalise before matching, and we document
            // that edits normalise line endings. Users who need CRLF preserved should use
            // `bash` + `sed`.
            let updated = {
                let mut s = String::with_capacity(
                    content.len() - normalised_old.len() + normalised_new.len(),
                );
                s.push_str(&content[..offset]);
                s.push_str(&normalised_new);
                s.push_str(&content[offset + normalised_old.len()..]);
                s
            };

            tokio::fs::write(&full_path, &updated)
                .await
                .map_err(|e| DomainError::Tool(format!("edit write failed: {}", e)))?;

            let diff = make_edit_diff(EditDiffArgs {
                path,
                content: &content,
                byte_offset: offset,
                old_text: &normalised_old,
                new_text: &normalised_new,
            });

            Ok(ToolResult {
                content: diff,
                is_error: false,
            })
        })
    }
}

/// Strip UTF-8 BOM and normalise CRLF → LF in a single pass.
fn fuzzy_normalise(s: &str) -> String {
    let s = s.strip_prefix('\u{FEFF}').unwrap_or(s);
    // Fast path: no CR bytes at all — skip all allocation
    if !s.contains('\r') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
            // consume following \n if present (CRLF → LF)
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(c);
        }
    }
    out
}

/// Count non-overlapping occurrences of `needle` in `haystack`, stopping at `cap`.
/// Returns `(count, first_match_offset)`.
fn count_occurrences_capped(haystack: &str, needle: &str, cap: usize) -> (usize, Option<usize>) {
    if needle.is_empty() {
        return (0, None);
    }
    let mut count = 0usize;
    let mut first_offset: Option<usize> = None;
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(needle) {
        let abs_pos = start + pos;
        if first_offset.is_none() {
            first_offset = Some(abs_pos);
        }
        count += 1;
        if count >= cap {
            return (count, first_offset);
        }
        start = abs_pos + needle.len();
    }
    (count, first_offset)
}

const DIFF_MAX_BYTES: usize = 4096;

struct EditDiffArgs<'a> {
    path: &'a str,
    content: &'a str,
    byte_offset: usize,
    old_text: &'a str,
    new_text: &'a str,
}

/// Produce a compact unified-diff-style snippet showing the change.
/// Returns a `@@` block with 2 lines of context around the change.
/// Output is capped at `DIFF_MAX_BYTES` to prevent prompt-injection via huge diffs.
fn make_edit_diff(a: EditDiffArgs<'_>) -> String {
    let EditDiffArgs {
        path,
        content,
        byte_offset,
        old_text,
        new_text,
    } = a;
    const CONTEXT: usize = 2;

    // Use byte counting (not lines().count()) to correctly handle trailing newlines
    let before = &content[..byte_offset];
    let start_line = before.bytes().filter(|&b| b == b'\n').count(); // 0-indexed
    let old_line_count = old_text.lines().count().max(1);
    let new_line_count = new_text.lines().count().max(1);

    // Collect only the window of lines we need — avoid materialising the whole file
    let total_lines = content.bytes().filter(|&b| b == b'\n').count() + 1;
    let ctx_start = start_line.saturating_sub(CONTEXT);
    let ctx_end = (start_line + old_line_count + CONTEXT).min(total_lines);

    let window_lines: Vec<&str> = content
        .lines()
        .skip(ctx_start)
        .take(ctx_end - ctx_start)
        .collect();

    let old_count = ctx_end - ctx_start;
    let new_count = old_count - old_line_count + new_line_count;
    let mut hunk = format!(
        "@@ -{},{} +{},{} @@\n",
        ctx_start + 1,
        old_count,
        ctx_start + 1,
        new_count
    );

    for (i, line) in window_lines.iter().enumerate() {
        let abs = ctx_start + i;
        if abs >= start_line && abs < start_line + old_line_count {
            hunk.push_str(&format!("-{}\n", line));
        } else {
            hunk.push_str(&format!(" {}\n", line));
        }
    }
    for ln in new_text.lines() {
        hunk.push_str(&format!("+{}\n", ln));
    }

    let diff_body = format!("Successfully edited {}\n\n```diff\n{}```", path, hunk);

    // Cap diff output to prevent oversized tool responses / prompt-injection surface
    if diff_body.len() > DIFF_MAX_BYTES {
        format!("Successfully edited {}", path)
    } else {
        diff_body
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
    async fn test_edit_replaces_unique_match() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("test.txt"), "hello world").unwrap();

        let tool = EditTool::new(ws, sb);
        let result = tool
            .execute(r#"{"path": "test.txt", "oldText": "hello", "newText": "goodbye"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(
            result.content.contains("@@"),
            "expected diff, got: {}",
            result.content
        );

        let content = std::fs::read_to_string(tmp.path().join("test.txt")).unwrap();
        assert_eq!(content, "goodbye world");
    }

    #[tokio::test]
    async fn test_edit_legacy_old_new_params() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("test.txt"), "hello world").unwrap();

        let tool = EditTool::new(ws, sb);
        // Backward-compat: old/new still accepted
        let result = tool
            .execute(r#"{"path": "test.txt", "old": "hello", "new": "goodbye"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn test_edit_substring_not_found() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("test.txt"), "hello").unwrap();

        let tool = EditTool::new(ws, sb);
        let result = tool
            .execute(r#"{"path": "test.txt", "oldText": "xyz", "newText": "abc"}"#)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_edit_rejects_ambiguous_match() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("dup.txt"), "x = 1\nx = 1").unwrap();

        let tool = EditTool::new(ws, sb);
        let result = tool
            .execute(r#"{"path": "dup.txt", "oldText": "x = 1", "newText": "x = 2"}"#)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(
            result.content.contains("2"),
            "expected count mention, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_edit_normalises_crlf() {
        let (ws, sb, tmp) = test_tools();
        // Write CRLF file
        std::fs::write(tmp.path().join("crlf.txt"), "hello\r\nworld").unwrap();

        let tool = EditTool::new(ws, sb);
        let result = tool
            .execute(r#"{"path": "crlf.txt", "oldText": "hello", "newText": "hi"}"#)
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);

        let content = std::fs::read_to_string(tmp.path().join("crlf.txt")).unwrap();
        assert!(
            content.contains("hi"),
            "expected replacement, got: {}",
            content
        );
    }

    #[tokio::test]
    async fn test_edit_strips_bom() {
        let (ws, sb, tmp) = test_tools();
        // Write file with UTF-8 BOM
        let bom_content = "\u{FEFF}hello world";
        std::fs::write(tmp.path().join("bom.txt"), bom_content).unwrap();

        let tool = EditTool::new(ws, sb);
        let result = tool
            .execute(r#"{"path": "bom.txt", "oldText": "hello", "newText": "hi"}"#)
            .await
            .unwrap();
        assert!(!result.is_error, "got error: {}", result.content);
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
    async fn test_read_offset_zero_is_error() {
        let (ws, sb, tmp) = test_tools();
        let tool = ReadTool::new(ws, sb);
        std::fs::write(tmp.path().join("zero.txt"), "content").unwrap();

        // offset=0 is invalid (1-indexed API)
        let result = tool.execute(r#"{"path": "zero.txt", "offset": 0}"#).await;
        assert!(
            result.is_err(),
            "expected error for offset=0 but got: {:?}",
            result.unwrap().content
        );
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
    async fn test_edit_rejects_oversized_file() {
        let (ws, sb, tmp) = test_tools();
        let tool = EditTool::new(ws, sb);

        let large_content = "a".repeat(1_048_577);
        std::fs::write(tmp.path().join("big-edit.txt"), large_content).unwrap();

        let result = tool
            .execute(r#"{"path": "big-edit.txt", "oldText": "a", "newText": "b"}"#)
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
