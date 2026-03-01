// ReadTool — Pi name: "read" (was "read_file")
// Supports text files with offset/limit pagination and image files as base64.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::path_utils::resolve_read_path;
use crate::infrastructure::tools::truncate::{
    DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES, format_size, truncate_head,
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

            // Image detection: if the file is a supported image type, return as base64.
            // Cap at 5 MiB — Anthropic's API rejects larger images in tool_result content.
            const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
            if let Some(mime) = detect_image_mime(&resolved) {
                if let Ok(meta) = tokio::fs::metadata(&resolved).await {
                    if meta.len() > MAX_IMAGE_BYTES {
                        let size = format_size(meta.len() as usize);
                        return Ok(ToolResult {
                            content: format!(
                                "Image is {size} — too large to send inline (max 5 MiB for API). \
                                 Describe what you need from the image instead.",
                            ),
                            is_error: true,
                            image_blocks: vec![],
                        });
                    }
                }
                let bytes = tokio::fs::read(&resolved)
                    .await
                    .map_err(|e| DomainError::Tool(format!("read image failed: {}", e)))?;
                use base64::Engine as _;
                let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
                let size = format_size(bytes.len());
                return Ok(ToolResult {
                    content: format!("Read image file [{}] ({size})", mime),
                    is_error: false,
                    image_blocks: vec![crate::domain::tool::ImageBlock {
                        mime_type: mime.to_string(),
                        data,
                    }],
                });
            }

            // Load file content
            let content = tokio::fs::read_to_string(&resolved)
                .await
                .map_err(|e| DomainError::Tool(format!("read failed: {}", e)))?;

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

/// Detect supported image MIME type by file extension (case-insensitive).
fn detect_image_mime(path: &Path) -> Option<&'static str> {
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
            detect_image_mime(Path::new("screenshot.png")),
            Some("image/png")
        );
    }

    #[test]
    fn test_detect_image_mime_jpg() {
        assert_eq!(
            detect_image_mime(Path::new("photo.jpg")),
            Some("image/jpeg")
        );
        assert_eq!(
            detect_image_mime(Path::new("photo.jpeg")),
            Some("image/jpeg")
        );
    }

    #[test]
    fn test_detect_image_mime_gif() {
        assert_eq!(detect_image_mime(Path::new("anim.gif")), Some("image/gif"));
    }

    #[test]
    fn test_detect_image_mime_webp() {
        assert_eq!(
            detect_image_mime(Path::new("icon.webp")),
            Some("image/webp")
        );
    }

    #[test]
    fn test_detect_image_mime_text_file() {
        assert_eq!(detect_image_mime(Path::new("notes.txt")), None);
        assert_eq!(detect_image_mime(Path::new("main.rs")), None);
        assert_eq!(detect_image_mime(Path::new("no_extension")), None);
    }

    #[test]
    fn test_detect_image_mime_uppercase_ext() {
        assert_eq!(detect_image_mime(Path::new("IMAGE.PNG")), Some("image/png"));
        assert_eq!(
            detect_image_mime(Path::new("Photo.JPG")),
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
}
