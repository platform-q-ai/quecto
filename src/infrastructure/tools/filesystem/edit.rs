// EditTool — Pi name: "edit" (was "edit_file")
// Fuzzy matching (BOM strip + CRLF normalisation), uniqueness check, diff output.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;

use super::resolve_and_validate;

const MAX_EDIT_FILE_BYTES: u64 = 1024 * 1024;
const DIFF_MAX_BYTES: usize = 4096;

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
            },"required":["path","oldText","newText"]}"#
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

        Box::pin(async move {
            let args: serde_json::Value =
                serde_json::from_str(&args_str).map_err(|e| DomainError::Tool(e.to_string()))?;
            let path = args["path"]
                .as_str()
                .ok_or_else(|| DomainError::Tool("missing 'path' argument".to_string()))?;
            // Accept "oldText" (Pi name) or legacy "old"
            let old_text = args["oldText"]
                .as_str()
                .or_else(|| args["old"].as_str())
                .ok_or_else(|| DomainError::Tool("missing 'oldText' argument".to_string()))?;
            // Accept "newText" (Pi name) or legacy "new"
            let new_text = args["newText"]
                .as_str()
                .or_else(|| args["new"].as_str())
                .ok_or_else(|| DomainError::Tool("missing 'newText' argument".to_string()))?;

            let full_path = resolve_and_validate(&workspace, &sandbox, path)?;
            enforce_edit_file_size_limit(&full_path).await?;

            let raw = tokio::fs::read_to_string(&full_path)
                .await
                .map_err(|e| DomainError::Tool(format!("edit read failed: {}", e)))?;

            let content = fuzzy_normalise(&raw);
            let normalised_old = fuzzy_normalise(old_text);

            let (count, match_offset) = count_occurrences_capped(&content, &normalised_old, 2);
            if count == 0 {
                return Ok(ToolResult {
                    content: format!("oldText not found in {}", path),
                    is_error: true,
                    image_blocks: vec![],
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
                    image_blocks: vec![],
                });
            }

            let offset = match_offset.expect("count=1 guarantees Some(offset)");
            let normalised_new = fuzzy_normalise(new_text);

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
                image_blocks: vec![],
            })
        })
    }
}

/// Strip UTF-8 BOM and normalise CRLF → LF in a single pass.
fn fuzzy_normalise(s: &str) -> String {
    let s = s.strip_prefix('\u{FEFF}').unwrap_or(s);
    if !s.contains('\r') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' {
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

struct EditDiffArgs<'a> {
    path: &'a str,
    content: &'a str,
    byte_offset: usize,
    old_text: &'a str,
    new_text: &'a str,
}

/// Produce a compact unified-diff-style snippet showing the change.
fn make_edit_diff(a: EditDiffArgs<'_>) -> String {
    let EditDiffArgs {
        path,
        content,
        byte_offset,
        old_text,
        new_text,
    } = a;
    const CONTEXT: usize = 2;

    let before = &content[..byte_offset];
    let start_line = before.bytes().filter(|&b| b == b'\n').count();
    let old_line_count = old_text.lines().count().max(1);
    let new_line_count = new_text.lines().count().max(1);

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

    if diff_body.len() > DIFF_MAX_BYTES {
        format!("Successfully edited {}", path)
    } else {
        diff_body
    }
}

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
    async fn test_edit_replaces_unique_match() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("test.txt"), "hello world").unwrap();
        let tool = EditTool::new(ws, sb);
        let result = tool
            .execute(r#"{"path": "test.txt", "oldText": "hello", "newText": "goodbye"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("@@"), "expected diff");
        let content = std::fs::read_to_string(tmp.path().join("test.txt")).unwrap();
        assert_eq!(content, "goodbye world");
    }

    #[tokio::test]
    async fn test_edit_legacy_old_new_params() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("f.txt"), "foo bar").unwrap();
        let tool = EditTool::new(ws, sb);
        let result = tool
            .execute(r#"{"path": "f.txt", "old": "foo", "new": "baz"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
        let content = std::fs::read_to_string(tmp.path().join("f.txt")).unwrap();
        assert_eq!(content, "baz bar");
    }

    #[tokio::test]
    async fn test_edit_substring_not_found() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("f.txt"), "hello world").unwrap();
        let tool = EditTool::new(ws, sb);
        let result = tool
            .execute(r#"{"path": "f.txt", "oldText": "xyz", "newText": "abc"}"#)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_edit_rejects_ambiguous_match() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("f.txt"), "aa aa").unwrap();
        let tool = EditTool::new(ws, sb);
        let result = tool
            .execute(r#"{"path": "f.txt", "oldText": "aa", "newText": "bb"}"#)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("matches"));
    }

    #[tokio::test]
    async fn test_edit_normalises_crlf() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("f.txt"), "line1\r\nline2\r\n").unwrap();
        let tool = EditTool::new(ws, sb);
        let result = tool
            .execute(r#"{"path": "f.txt", "oldText": "line1\nline2", "newText": "replaced"}"#)
            .await
            .unwrap();
        assert!(!result.is_error, "got: {}", result.content);
    }

    #[tokio::test]
    async fn test_edit_strips_bom() {
        let (ws, sb, tmp) = test_tools();
        let bom_content = "\u{FEFF}hello world";
        std::fs::write(tmp.path().join("f.txt"), bom_content).unwrap();
        let tool = EditTool::new(ws, sb);
        let result = tool
            .execute(r#"{"path": "f.txt", "oldText": "hello", "newText": "hi"}"#)
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "BOM should be stripped: {}",
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
