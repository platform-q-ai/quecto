// ReadTool — tool name: "read"
// Supports text files with offset/limit pagination and image files as base64.

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::tool::{ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::path_utils::resolve_read_path;
use crate::infrastructure::tools::truncate::{
    DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, TruncatedBy, format_size,
};

use super::fs_failure::{Access, explain, not_utf8_text, refused};
use super::{MAX_READ_BYTES, shell_escape_single};

pub struct ReadTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
    cache: Arc<Mutex<ReadCache>>,
}

#[derive(Default)]
struct ReadCache {
    entries: HashMap<ReadCacheKey, ReadCacheEntry>,
    /// The canonical file each requested `path` argument resolved to when
    /// its delivery was cached (#2348): a collapse notice finds the file
    /// without touching the filesystem.
    resolved: HashMap<String, PathBuf>,
    next_sequence: u64,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ReadCacheKey {
    path: PathBuf,
    offset: Option<usize>,
    limit: Option<usize>,
}

struct ReadCacheEntry {
    hash: String,
    line_count: usize,
    sequence: u64,
}

impl ReadTool {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self {
            workspace,
            sandbox,
            cache: Arc::new(Mutex::new(ReadCache::default())),
        }
    }
}

impl Tool for ReadTool {
    /// Reads only: calls may overlap (#2169).
    fn overlaps_safely(&self, _arguments: &str) -> bool {
        true
    }

    /// The model no longer holds this read's content in full (#2348 review
    /// M1): forget every cached delivery of its file, so the next read
    /// answers the content instead of the unchanged marker.
    fn result_collapsed(&self, arguments: &str) {
        let Some(path) = serde_json::from_str::<serde_json::Value>(arguments)
            .ok()
            .and_then(|args| {
                args.get("path")
                    .and_then(|p| p.as_str())
                    .map(str::to_string)
            })
        else {
            return;
        };
        // No filesystem call here: the notice runs on the async pruning
        // path, so the file is the one this argument resolved to when read.
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let Some(cache_path) = cache.resolved.get(&path).cloned() else {
            return;
        };
        cache.entries.retain(|key, _| key.path != cache_path);
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read".into(),
            description: "Read the contents of a file. Supports text files and images (jpg, png, gif, webp). Images are sent as attachments. Unchanged repeated same-scope text reads may return a short marker; pass force:true to return content. For text files, output is truncated to 2000 lines or 50KB (whichever is hit first). Use offset/limit for large files. When you need the full file, continue with offset until complete. Example: {\"path\": \"src/main.rs\"}".into(),
            parameters_schema: r#"{"type":"object","properties":{"path":{"type":"string","description":"Path to the file to read (relative or absolute)"},"offset":{"type":"number","description":"Line number to start reading from (1-indexed)"},"limit":{"type":"number","description":"Maximum number of lines to read"},"force":{"type":"boolean","description":"Bypass unchanged-file cache and return current text content"}},"required":["path"]}"#.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args: Result<serde_json::Value, _> = serde_json::from_str(arguments);
        let workspace = self.workspace.clone();
        let sandbox = self.sandbox.clone();
        let cache = self.cache.clone();

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
                        delivery_metadata: None,
                    });
                }
            };
            let Some(path) = args["path"].as_str() else {
                return Ok(ToolResult {
                    content: "missing 'path' argument. Example: {\"path\": \"src/main.rs\"}"
                        .to_string(),
                    is_error: true,
                    image_blocks: vec![],
                    delivery_metadata: None,
                });
            };

            // Resolve using read-path (macOS filename variant probing)
            let resolved = resolve_read_path(path, &workspace);
            let validated_str = resolved.to_string_lossy().to_string();
            sandbox
                .validate_path(&validated_str)
                .map_err(|e| DomainError::Security(e.to_string()))?;
            let cache_path = tokio::fs::canonicalize(&resolved)
                .await
                .unwrap_or_else(|_| resolved.clone());

            // Parse optional offset (1-indexed) and limit. Models often emit integral
            // JSON floats (e.g. 370.0) for schema "number"; honor those and
            // hard-error on invalid paging args instead of silent default-head.
            let offset = match args.get("offset") {
                None => None,
                Some(v) if v.is_null() => None,
                Some(v) => match parse_optional_usize_arg(v, "offset") {
                    Ok(offset) => offset,
                    Err(message) => return Ok(refused(message)),
                },
            };
            let limit = match args.get("limit") {
                None => None,
                Some(v) if v.is_null() => None,
                Some(v) => match parse_optional_usize_arg(v, "limit") {
                    Ok(limit) => limit,
                    Err(message) => return Ok(refused(message)),
                },
            };
            let force = args.get("force").and_then(|v| v.as_bool()).unwrap_or(false);

            // Safety cap: reject reads > 10 MiB before loading into memory.
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
                        delivery_metadata: None,
                    });
                }
            }

            // Read entire file once (up to 10 MiB cap, already checked above).
            // Peek magic bytes from the buffer — avoids TOCTOU and extra syscalls.
            let raw_bytes = match tokio::fs::read(&resolved).await {
                Ok(bytes) => bytes,
                Err(error) => return read_failure(&resolved, path, &error).await,
            };

            // Magic-byte MIME detection. Extension-only fallback is intentionally
            // absent — text files named .jpg should be read as text. What an
            // image is, and the limit, are `quecto_image`'s (#2422): a file is
            // an image when its signature names a type and its header is
            // readable. Anything else (a JPEG with stray bytes, an Apple CgBI
            // PNG, text that starts "GIF89a") is read as text or binary below.
            let image = quecto_image::ImageMime::sniff(&raw_bytes)
                .filter(|mime| quecto_image::dimensions_of_bytes(*mime, &raw_bytes).is_some());
            if let Some(mime) = image {
                return Ok(image_result(mime, &raw_bytes));
            }
            // Not an image — interpret as UTF-8 text; anything else is named
            // as binary (#2166).
            let content = match String::from_utf8(raw_bytes) {
                Ok(content) => content,
                Err(error) => {
                    return Ok(refused(not_utf8_text(Access::Read, path, error.as_bytes())));
                }
            };

            // A paging argument the file cannot honour is refused (#2189).
            let selected = match select_read_text(&content, offset, limit) {
                Ok(selected) => selected,
                Err(DomainError::Tool(message)) => return Ok(refused(message)),
                Err(other) => return Err(other),
            };
            let hash = sha256_hex(selected.as_bytes());
            let line_count = selected.lines().count();
            let key = ReadCacheKey {
                path: cache_path,
                offset: Some(offset.unwrap_or(1)),
                limit,
            };

            if !force {
                let cache_guard = cache
                    .lock()
                    .map_err(|_| DomainError::Tool("read cache lock poisoned".to_string()))?;
                if let Some(entry) = cache_guard.entries.get(&key) {
                    if entry.hash == hash {
                        return Ok(ToolResult {
                            content: format!(
                                "[unchanged since read {}, hash match, {} lines; if you no \
                                 longer have its full content, pass force:true]",
                                entry.sequence, entry.line_count
                            ),
                            is_error: false,
                            image_blocks: vec![],
                            delivery_metadata: None,
                        });
                    }
                }
            }

            let output = apply_read_truncation(&content, path, offset, limit)?;
            update_read_cache(cache, path, key, hash, line_count)?;

            Ok(ToolResult {
                content: output,
                is_error: false,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

/// Parse optional line-count tool arg from an already-decoded JSON value.
///
/// - missing / null is handled by the caller (not passed here)
/// - finite, non-negative, integral, in 0..=usize::MAX → Ok(Some(n))
///   (0 still allowed here; apply_read_truncation rejects offset/limit 0)
/// - anything else → Err(message)
///
/// Never truncates via unchecked `as usize`.
fn parse_optional_usize_arg(
    value: &serde_json::Value,
    name: &str,
) -> Result<Option<usize>, String> {
    match value {
        serde_json::Value::Number(n) => {
            // Integer fast path with explicit usize fit check (no u64→usize truncation).
            if let Some(u) = n.as_u64() {
                return match usize::try_from(u) {
                    Ok(v) => Ok(Some(v)),
                    // Unreachable on 64-bit hosts; required on 32-bit where u64 can exceed usize.
                    Err(_) => Err(format!(
                        "invalid '{name}': value {u} is out of range for this platform (max {})",
                        usize::MAX
                    )),
                };
            }
            // Reject negative integers early with a clear message.
            if let Some(i) = n.as_i64() {
                return Err(format!(
                    "invalid '{name}': expected a non-negative integer, got {i}"
                ));
            }
            // Remaining JSON numbers are floats (serde_json only stores finite f64 here).
            // as_u64/as_i64 already failed, so as_f64 must succeed for a Number.
            let f = n.as_f64().expect("JSON Number without u64/i64 must be f64");
            if f < 0.0 {
                return Err(format!(
                    "invalid '{name}': expected a non-negative integer, got {f}"
                ));
            }
            // Reject anything outside the exact u64 domain before casting.
            // `usize::MAX as f64` rounds UP to 2^64 on 64-bit hosts, so a bound
            // of `f > (usize::MAX as f64)` would accept 2^64 and then saturate
            // via `f as u64` → u64::MAX. 2^64 is exact in f64; every finite f64
            // ≥ 2^64 is outside both u64 and usize.
            const U64_MAX_PLUS_ONE: f64 = 18446744073709551616.0; // 2^64, exact
            if f >= U64_MAX_PLUS_ONE {
                return Err(format!(
                    "invalid '{name}': value {f} is out of range for this platform (max {})",
                    usize::MAX
                ));
            }
            // Integral relative to the already-parsed float — no rounding.
            // After the 2^64 gate, `f as u64` is a non-saturating conversion for
            // integral values (non-integral still fail the equality check).
            let as_u = f as u64;
            if f != as_u as f64 {
                return Err(format!(
                    "invalid '{name}': expected an integer line count, got {f}"
                ));
            }
            // Explicit usize fit (required on 32-bit; identity on 64-bit).
            match usize::try_from(as_u) {
                Ok(v) => Ok(Some(v)),
                Err(_) => Err(format!(
                    "invalid '{name}': value {as_u} is out of range for this platform (max {})",
                    usize::MAX
                )),
            }
        }
        other => Err(format!("invalid '{name}': expected a number, got {other}")),
    }
}

/// The result of reading the `mime` file `bytes`: the image admitted as an
/// attachment (#2422), or why it cannot be sent.
fn image_result(mime: quecto_image::ImageMime, bytes: &[u8]) -> ToolResult {
    let size = format_size(bytes.len());
    // `from_bytes` makes the one size check.
    match quecto_image::ImageAttachment::from_bytes(mime, bytes) {
        Ok(image) => ToolResult {
            content: format!("Read image file [{}] ({size})", image.mime_type()),
            is_error: false,
            image_blocks: vec![image.into()],
            delivery_metadata: None,
        },
        Err(refusal @ quecto_image::ImageRefusal::TooLarge) => ToolResult {
            content: format!(
                "Image is {size} — too large to send inline: {refusal}. \
                 Describe what you need from the image instead.",
            ),
            is_error: true,
            image_blocks: vec![],
            delivery_metadata: None,
        },
        Err(refusal) => ToolResult {
            content: format!("Image file is {refusal}; it cannot be sent."),
            is_error: true,
            image_blocks: vec![],
            delivery_metadata: None,
        },
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn update_read_cache(
    cache: Arc<Mutex<ReadCache>>,
    requested: &str,
    key: ReadCacheKey,
    hash: String,
    line_count: usize,
) -> Result<(), DomainError> {
    let mut cache = cache
        .lock()
        .map_err(|_| DomainError::Tool("read cache lock poisoned".to_string()))?;
    let sequence = match cache.entries.get(&key) {
        Some(entry) if entry.hash == hash => entry.sequence,
        _ => {
            cache.next_sequence += 1;
            cache.next_sequence
        }
    };
    cache
        .resolved
        .insert(requested.to_string(), key.path.clone());
    cache.entries.insert(
        key,
        ReadCacheEntry {
            hash,
            line_count,
            sequence,
        },
    );
    Ok(())
}

fn select_read_text(
    content: &str,
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

    let total_lines = content.lines().count();
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
    let start_byte = byte_offset_for_line(content, start_line);
    let max_lines = limit.unwrap_or(usize::MAX);
    let end_byte = byte_offset_after_lines(&content[start_byte..], max_lines)
        .map(|relative| start_byte + relative)
        .unwrap_or(content.len());
    Ok(content[start_byte..end_byte].to_string())
}

fn byte_offset_for_line(content: &str, zero_indexed_line: usize) -> usize {
    if zero_indexed_line == 0 {
        return 0;
    }
    content
        .match_indices('\n')
        .nth(zero_indexed_line - 1)
        .map(|(idx, _)| idx + 1)
        .unwrap_or(content.len())
}

fn byte_offset_after_lines(content: &str, line_count: usize) -> Option<usize> {
    if line_count == usize::MAX {
        return None;
    }
    if line_count == 0 {
        return Some(0);
    }
    content
        .match_indices('\n')
        .nth(line_count - 1)
        .map(|(idx, _)| idx + 1)
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

/// Why a read failed, named and with a next step (#2166, #2189).
async fn read_failure(
    full_path: &std::path::Path,
    path: &str,
    error: &std::io::Error,
) -> Result<ToolResult, DomainError> {
    Ok(refused(explain(Access::Read, full_path, path, error).await))
}

#[cfg(test)]
#[path = "read_cache_tests.rs"]
mod cache_tests;
#[cfg(test)]
#[path = "read_tests.rs"]
mod tests;
