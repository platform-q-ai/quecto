// ReadTool — Pi name: "read"
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
    DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, TruncatedBy, format_size, truncate_head,
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
                Err(e) => return Ok(ToolResult {
                    content: format!("invalid JSON arguments: {e}. Example: {{\"path\": \"src/main.rs\"}}"),
                    is_error: true,
                    image_blocks: vec![],
                }),
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

/// Detect supported image MIME type by file extension (case-insensitive).
/// Production path uses magic-byte detection only; this helper is test-only.
#[cfg(test)]
fn detect_image_mime_by_ext(path: &std::path::Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
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

    let tr = truncate_head(&sliced, max_lines, DEFAULT_MAX_BYTES);

    let mut output = String::new();

    if tr.first_line_exceeds_limit {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::security::sandbox::Sandbox;
    use std::path::Path;
    use tempfile::TempDir;

    fn test_tools() -> (Arc<PathBuf>, Arc<Sandbox>, TempDir) {
        let tmp = TempDir::new().unwrap();
        let workspace = Arc::new(tmp.path().to_path_buf());
        let sandbox = Arc::new(Sandbox::new(Some(tmp.path().to_path_buf()), true));
        (workspace, sandbox, tmp)
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

        let content: String = (1..=3000).map(|i| format!("line{}\n", i)).collect();
        std::fs::write(tmp.path().join("big.txt"), &content).unwrap();

        let result = tool.execute(r#"{"path": "big.txt"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(
            result.content.contains("Showing lines"),
            "expected truncation hint"
        );
    }

    #[tokio::test]
    async fn test_read_offset_pagination() {
        let (ws, sb, tmp) = test_tools();
        let tool = ReadTool::new(ws, sb);
        let content: String = (1..=10).map(|i| format!("line{}\n", i)).collect();
        std::fs::write(tmp.path().join("paged.txt"), &content).unwrap();

        let result = tool
            .execute(r#"{"path": "paged.txt", "offset": 5, "limit": 3}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("line5"));
        assert!(result.content.contains("line7"));
        assert!(!result.content.contains("line4"));
        assert!(!result.content.contains("line8"));
    }

    #[tokio::test]
    async fn test_read_offset_beyond_eof_error() {
        let (ws, sb, tmp) = test_tools();
        let tool = ReadTool::new(ws, sb);
        std::fs::write(tmp.path().join("small.txt"), "one\ntwo\n").unwrap();

        let result = tool
            .execute(r#"{"path": "small.txt", "offset": 999}"#)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_read_offset_zero_is_error() {
        let (ws, sb, tmp) = test_tools();
        let tool = ReadTool::new(ws, sb);
        std::fs::write(tmp.path().join("f.txt"), "hello").unwrap();

        let result = tool.execute(r#"{"path": "f.txt", "offset": 0}"#).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("1-indexed"));
    }

    // --- detect_image_mime unit tests ---

    #[test]
    fn test_detect_image_mime_png() {
        assert_eq!(
            detect_image_mime_by_ext(Path::new("screenshot.png")),
            Some("image/png")
        );
    }

    #[test]
    fn test_detect_image_mime_jpg() {
        assert_eq!(
            detect_image_mime_by_ext(Path::new("photo.jpg")),
            Some("image/jpeg")
        );
        assert_eq!(
            detect_image_mime_by_ext(Path::new("photo.jpeg")),
            Some("image/jpeg")
        );
    }

    #[test]
    fn test_detect_image_mime_gif() {
        assert_eq!(
            detect_image_mime_by_ext(Path::new("anim.gif")),
            Some("image/gif")
        );
    }

    #[test]
    fn test_detect_image_mime_webp() {
        assert_eq!(
            detect_image_mime_by_ext(Path::new("icon.webp")),
            Some("image/webp")
        );
    }

    #[test]
    fn test_detect_image_mime_text_file() {
        assert_eq!(detect_image_mime_by_ext(Path::new("notes.txt")), None);
        assert_eq!(detect_image_mime_by_ext(Path::new("main.rs")), None);
        assert_eq!(detect_image_mime_by_ext(Path::new("no_extension")), None);
    }

    #[test]
    fn test_detect_image_mime_uppercase_ext() {
        assert_eq!(
            detect_image_mime_by_ext(Path::new("IMAGE.PNG")),
            Some("image/png")
        );
        assert_eq!(
            detect_image_mime_by_ext(Path::new("Photo.JPG")),
            Some("image/jpeg")
        );
    }

    #[tokio::test]
    async fn test_read_png_returns_image_block() {
        let (ws, sb, tmp) = test_tools();
        let tool = ReadTool::new(ws, sb);
        let png_bytes: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE,
        ];
        std::fs::write(tmp.path().join("img.png"), png_bytes).unwrap();

        let result = tool.execute(r#"{"path": "img.png"}"#).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(result.image_blocks.len(), 1);
        assert_eq!(result.image_blocks[0].mime_type, "image/png");
        assert!(!result.image_blocks[0].data.is_empty());
        assert!(result.content.contains("image/png"));
    }

    #[tokio::test]
    async fn test_read_text_file_has_no_image_blocks() {
        let (ws, sb, tmp) = test_tools();
        let tool = ReadTool::new(ws, sb);
        std::fs::write(tmp.path().join("hello.txt"), "hello world").unwrap();
        let result = tool.execute(r#"{"path": "hello.txt"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.image_blocks.is_empty());
        assert!(result.content.contains("hello world"));
    }

    // --- Magic-byte MIME detection ---

    #[test]
    fn test_detect_mime_by_magic_png() {
        const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0];
        assert_eq!(detect_mime_by_magic(PNG), Some("image/png"));
    }

    #[test]
    fn test_detect_mime_by_magic_jpeg() {
        const JPEG: &[u8] = &[0xFF, 0xD8, 0xFF, 0xD9];
        assert_eq!(detect_mime_by_magic(JPEG), Some("image/jpeg"));
    }

    #[test]
    fn test_detect_mime_by_magic_gif89a() {
        assert_eq!(detect_mime_by_magic(b"GIF89a\x01"), Some("image/gif"));
    }

    #[test]
    fn test_detect_mime_by_magic_gif87a() {
        assert_eq!(detect_mime_by_magic(b"GIF87a\x01"), Some("image/gif"));
    }

    #[test]
    fn test_detect_mime_by_magic_webp() {
        let webp = b"RIFF\x20\x00\x00\x00WEBPVP8L";
        assert_eq!(detect_mime_by_magic(webp), Some("image/webp"));
    }

    #[test]
    fn test_detect_mime_by_magic_text() {
        assert_eq!(detect_mime_by_magic(b"hello world"), None);
        assert_eq!(detect_mime_by_magic(b""), None);
    }

    #[tokio::test]
    async fn test_magic_bytes_detect_png_no_extension() {
        let (ws, sb, tmp) = test_tools();
        let tool = ReadTool::new(ws, sb);
        // Valid PNG magic bytes but no file extension
        let png_bytes: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52,
        ];
        std::fs::write(tmp.path().join("screenshot"), png_bytes).unwrap();
        let result = tool.execute(r#"{"path": "screenshot"}"#).await.unwrap();
        assert!(
            !result.is_error,
            "magic byte PNG should be detected: {}",
            result.content
        );
        assert!(
            result
                .image_blocks
                .iter()
                .any(|b| b.mime_type == "image/png")
        );
    }

    #[tokio::test]
    async fn test_magic_bytes_detect_jpeg_wrong_extension() {
        let (ws, sb, tmp) = test_tools();
        let tool = ReadTool::new(ws, sb);
        // JPEG magic bytes with .dat extension
        let jpeg_bytes: &[u8] = &[0xFF, 0xD8, 0xFF, 0xD9];
        std::fs::write(tmp.path().join("photo.dat"), jpeg_bytes).unwrap();
        let result = tool.execute(r#"{"path": "photo.dat"}"#).await.unwrap();
        assert!(
            !result.is_error,
            "magic byte JPEG should be detected: {}",
            result.content
        );
        assert!(
            result
                .image_blocks
                .iter()
                .any(|b| b.mime_type == "image/jpeg")
        );
    }

    #[tokio::test]
    async fn test_text_jpg_not_detected_as_image() {
        let (ws, sb, tmp) = test_tools();
        let tool = ReadTool::new(ws, sb);
        // Text content with .jpg extension — magic bytes are not image bytes
        std::fs::write(tmp.path().join("not_image.jpg"), "hello world").unwrap();
        let result = tool.execute(r#"{"path": "not_image.jpg"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(
            result.image_blocks.is_empty(),
            "text file should not produce image block"
        );
        assert!(result.content.contains("hello world"));
    }

    // --- Truncation notice format ---

    #[test]
    fn test_truncation_byte_limit_includes_50kb_hint() {
        // Content > 50KB in multi-line format; should trigger byte truncation notice.
        // Each line is ~50 bytes; 1500 lines = ~75KB total, truncation happens by bytes.
        let line = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuv\n"; // 50 bytes
        let content: String = line.repeat(1500); // ~75KB
        let result = apply_read_truncation(&content, "f.txt", None, None).unwrap();
        assert!(
            result.contains("50KB limit"),
            "byte-truncated notice should include '50KB limit', got: {}",
            &result[result.len().saturating_sub(200)..]
        );
    }

    #[test]
    fn test_truncation_user_limit_shows_more_lines() {
        // 100 lines, user requests 10 → "N more lines in file" notice.
        let content: String = (1..=100).map(|i| format!("line{}\n", i)).collect();
        let result = apply_read_truncation(&content, "f.txt", None, Some(10)).unwrap();
        assert!(
            result.contains("more lines in file"),
            "user-limit notice should say 'more lines in file', got: {}",
            &result[result.len().saturating_sub(200)..]
        );
    }

    #[tokio::test]
    async fn test_image_read_no_resize_note() {
        let (ws, sb, tmp) = test_tools();
        let tool = ReadTool::new(ws, sb);
        // Minimal valid 1×1 PNG
        let png_bytes: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE,
        ];
        std::fs::write(tmp.path().join("small.png"), png_bytes).unwrap();
        let result = tool.execute(r#"{"path": "small.png"}"#).await.unwrap();
        assert!(!result.is_error);
        assert_eq!(
            result.image_blocks.len(),
            1,
            "should return one image block"
        );
        assert!(
            !result.content.contains("resized"),
            "should not mention resize, got: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_read_empty_object_returns_actionable_error() {
        let (ws, sb, _tmp) = test_tools();
        let tool = ReadTool::new(ws, sb);
        let result = tool.execute("{}").await.unwrap();
        assert!(result.is_error, "expected error, got: {}", result.content);
        assert!(
            result.content.contains("path"),
            "should mention 'path', got: {}",
            result.content
        );
        assert!(
            result.content.contains("Example"),
            "should include example, got: {}",
            result.content
        );
    }

    #[test]
    fn test_read_description_includes_example() {
        let (ws, sb, _tmp) = test_tools();
        let tool = ReadTool::new(ws, sb);
        let def = tool.definition();
        assert!(
            def.description.contains("Example"),
            "read description should include Example, got: {}",
            def.description
        );
    }
}
