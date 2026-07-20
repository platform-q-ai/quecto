// ReadTool — tool name: "read"
// Supports text files with offset/limit pagination and image files as base64.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::path_utils::resolve_read_path;
use crate::infrastructure::tools::truncate::{
    DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, TruncatedBy, format_size,
};

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
            name: "read".into(),
            description: "Read the contents of a file. Supports text files and images (jpg, png, gif, webp). Images are sent as attachments. For text files, output is truncated to 2000 lines or 50KB (whichever is hit first). Use offset/limit for large files. When you need the full file, continue with offset until complete. Example: {\"path\": \"src/main.rs\"}".into(),
            parameters_schema: r#"{"type":"object","properties":{"path":{"type":"string","description":"Path to the file to read (relative or absolute)"},"offset":{"type":"number","description":"Line number to start reading from (1-indexed)"},"limit":{"type":"number","description":"Maximum number of lines to read"}},"required":["path"]}"#.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args: Result<serde_json::Value, _> = serde_json::from_str(arguments);
        let workspace = self.workspace.clone();
        let sandbox = self.sandbox.clone();

        Box::pin(async move {
            // LLM-addressable: malformed JSON → ToolResult { is_error: true }
            // so the LLM can read the parser's message and retry with valid
            // input, per the Tool port's error-handling contract.
            let args = match args {
                Ok(v) => v,
                Err(e) => {
                    return Ok(ToolResult {
                        content: format!(
                            "invalid JSON arguments: {e}. Example: {{\"path\": \"src/main.rs\"}}"
                        ),
                        is_error: true,
                        image_blocks: vec![],
                    });
                }
            };
            let Some(path) = args["path"].as_str() else {
                return Ok(ToolResult {
                    content: "missing 'path' argument. Example: {\"path\": \"src/main.rs\"}"
                        .to_string(),
                    is_error: true,
                    image_blocks: vec![],
                });
            };

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
                        image_blocks: vec![],
                    });
                }
            }

            // Read entire file once (up to 10 MiB cap, already checked above).
            // Peek magic bytes from the buffer — avoids TOCTOU and extra syscalls.
            const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
            let raw_bytes = tokio::fs::read(&resolved)
                .await
                .map_err(|e| DomainError::Tool(format!("read failed: {}", e)))?;

            // Magic-byte MIME detection. Extension-only fallback is intentionally
            // absent — text files named .jpg should be read as text.
            if let Some(orig_mime) = detect_mime_by_magic(&raw_bytes) {
                if raw_bytes.len() > MAX_IMAGE_BYTES {
                    let size = format_size(raw_bytes.len());
                    return Ok(ToolResult {
                        content: format!(
                            "Image is {size} — too large to send inline (max 5 MiB for API). \
                             Describe what you need from the image instead.",
                        ),
                        is_error: true,
                        image_blocks: vec![],
                    });
                }
                use base64::Engine as _;
                let data = base64::engine::general_purpose::STANDARD.encode(&raw_bytes);
                let size = format_size(raw_bytes.len());
                let content = format!("Read image file [{}] ({size})", orig_mime);
                return Ok(ToolResult {
                    content,
                    is_error: false,
                    image_blocks: vec![crate::domain::tool::ImageBlock {
                        mime_type: orig_mime,
                        data,
                    }],
                });
            }

            // Not an image — interpret as UTF-8 text.
            let content = String::from_utf8(raw_bytes)
                .map_err(|e| DomainError::Tool(format!("read failed (not valid UTF-8): {}", e)))?;

            let output = apply_read_truncation(&content, path, offset, limit)?;

            Ok(ToolResult {
                content: output,
                is_error: false,
                image_blocks: vec![],
            })
        })
    }
}

/// Wrap a path in single quotes for use in a shell command hint.
fn shell_escape_single(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

/// Detect supported image MIME type by file magic bytes.
///
/// Checks the first few bytes of the file content against known image signatures.
/// Returns `None` if no known image signature is found.
///
/// Signatures checked:
/// - PNG:  `\x89PNG\r\n\x1a\n` (8 bytes)
/// - JPEG: `\xFF\xD8\xFF` (3 bytes)
/// - GIF:  `GIF87a` or `GIF89a` (6 bytes)
/// - WebP: `RIFF....WEBP` (12 bytes)
fn detect_mime_by_magic(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 8 && bytes[..8] == [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A] {
        return Some("image/png");
    }
    if bytes.len() >= 3 && bytes[..3] == [0xFF, 0xD8, 0xFF] {
        return Some("image/jpeg");
    }
    if bytes.len() >= 6 && (bytes[..6] == *b"GIF87a" || bytes[..6] == *b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.len() >= 12 && bytes[..4] == *b"RIFF" && bytes[8..12] == *b"WEBP" {
        return Some("image/webp");
    }
    None
}

/// Apply offset/limit pagination and head-truncation to text file content.
///
/// # Offset semantics
/// - `None` → start from line 1
/// - `Some(0)` → **error** (1-indexed; 0 is not valid)
/// - `Some(n)` → start from line n (1-indexed)
fn apply_read_truncation(
    content: &str,
    path: &str,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<String, DomainError> {
    if offset == Some(0) {
        return Err(DomainError::Tool(
            "offset is 1-indexed; 0 is not valid. Use offset=1 for the first line.".to_string(),
        ));
    }
    if limit == Some(0) {
        return Err(DomainError::Tool(
            "limit must be at least 1 when provided.".to_string(),
        ));
    }

    let total_lines: usize = content.lines().count();

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

    let max_lines = limit.unwrap_or(DEFAULT_MAX_LINES);

    let tr = truncate_head_from_offset(content, start_line, max_lines, DEFAULT_MAX_BYTES);

    let mut output = String::new();

    if tr.first_line_exceeds_limit {
        let line_size = format_size(tr.first_line_bytes);
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
                // User provided an explicit limit — tell them how many lines remain.
                output.push_str(&format!(
                    "\n[{} more lines in file. Use offset={} to continue.]",
                    remaining, next_offset
                ));
            } else if tr.truncated_by == Some(TruncatedBy::Bytes) {
                // Auto-truncation by byte limit — include the "(50KB limit)" hint.
                output.push_str(&format!(
                    "\n[Showing lines {}-{} of {} (50KB limit). Use offset={} to continue.]",
                    shown_start, shown_end, total_lines, next_offset
                ));
            } else {
                // Auto-truncation by line limit.
                output.push_str(&format!(
                    "\n[Showing lines {}-{} of {}. Use offset={} to continue.]",
                    shown_start, shown_end, total_lines, next_offset
                ));
            }
        }
    }

    Ok(output)
}

struct OffsetHeadTruncation {
    content: String,
    truncated: bool,
    truncated_by: Option<TruncatedBy>,
    output_lines: usize,
    first_line_exceeds_limit: bool,
    first_line_bytes: usize,
}

fn truncate_head_from_offset(
    content: &str,
    start_line: usize,
    max_lines: usize,
    max_bytes: usize,
) -> OffsetHeadTruncation {
    let mut output = String::new();
    let mut output_lines = 0usize;
    let mut output_bytes = 0usize;
    let mut first_line_bytes = 0usize;
    let mut truncated_by = None;
    let mut first_line_exceeds_limit = false;

    for line in content.lines().skip(start_line) {
        if output_lines == 0 {
            first_line_bytes = line.len();
        }
        if output_lines >= max_lines {
            truncated_by = Some(TruncatedBy::Lines);
            break;
        }

        let separator_bytes = usize::from(output_lines > 0);
        let would_be = output_bytes + separator_bytes + line.len();
        if would_be > max_bytes {
            truncated_by = Some(TruncatedBy::Bytes);
            first_line_exceeds_limit = output_lines == 0;
            break;
        }

        if separator_bytes == 1 {
            output.push('\n');
        }
        output.push_str(line);
        output_bytes = would_be;
        output_lines += 1;
    }

    OffsetHeadTruncation {
        content: output,
        truncated: truncated_by.is_some(),
        truncated_by,
        output_lines,
        first_line_exceeds_limit,
        first_line_bytes,
    }
}

#[cfg(test)]
#[path = "read_tests.rs"]
mod tests;
