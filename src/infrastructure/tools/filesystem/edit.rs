// EditTool — Pi name: "edit" (was "edit_file")
// Two-stage exact→fuzzy matching, CRLF/BOM preservation, no-op detection,
// LCS-based unified diff via the `similar` crate.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use similar::{ChangeTag, TextDiff};

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;

use super::resolve_and_validate;

const MAX_EDIT_FILE_BYTES: u64 = 1024 * 1024;
const DIFF_MAX_BYTES: usize = 4096;
const DIFF_CONTEXT_LINES: usize = 4;

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
                          (including whitespace). Use this for precise, surgical edits. \
                          Example: {\"path\": \"file.txt\", \"oldText\": \"old\", \"newText\": \"new\"}"
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
            let edit_example =
                "{\"path\": \"file.txt\", \"oldText\": \"old\", \"newText\": \"new\"}";
            let Some(path) = args["path"].as_str() else {
                return Ok(ToolResult {
                    content: format!("missing 'path' argument. Example: {}", edit_example),
                    is_error: true,
                    image_blocks: vec![],
                });
            };
            // Accept "oldText" (Pi name) or legacy "old"
            let old_text = match args["oldText"].as_str().or_else(|| args["old"].as_str()) {
                Some(v) => v,
                None => {
                    return Ok(ToolResult {
                        content: format!("missing 'oldText' argument. Example: {}", edit_example),
                        is_error: true,
                        image_blocks: vec![],
                    });
                }
            };
            // Accept "newText" (Pi name) or legacy "new"
            let new_text = match args["newText"].as_str().or_else(|| args["new"].as_str()) {
                Some(v) => v,
                None => {
                    return Ok(ToolResult {
                        content: format!("missing 'newText' argument. Example: {}", edit_example),
                        is_error: true,
                        image_blocks: vec![],
                    });
                }
            };

            let full_path = resolve_and_validate(&workspace, &sandbox, path)?;
            enforce_edit_file_size_limit(&full_path).await?;

            // Read raw bytes so we can preserve BOM and detect original line endings.
            let raw = tokio::fs::read_to_string(&full_path)
                .await
                .map_err(|e| DomainError::Tool(format!("edit read failed: {}", e)))?;

            // Detect original line ending BEFORE normalisation.
            let original_ending = detect_line_ending(&raw);
            // Detect BOM BEFORE normalisation.
            let has_bom = raw.starts_with('\u{FEFF}');

            // Stage 1: exact match on BOM-stripped / CRLF-normalised content.
            // `content` is what we splice into — the base-normalised file.
            let content = base_normalise(&raw);
            let base_old = base_normalise(old_text);

            let (count, match_offset) = count_occurrences_capped(&content, &base_old, 2);

            // Stage 2: fuzzy fallback — locate match only; splice into `content`.
            // fuzzy_char() is a 1:1 char→char substitution so byte offsets
            // in fuzzy_content correspond to the same positions in `content`.
            let (splice_old, splice_offset, splice_count) = if count == 0 {
                let fuzzy_content = normalize_for_fuzzy_match(&content);
                let fuzzy_old = normalize_for_fuzzy_match(old_text);
                let (fc, fo) = count_occurrences_capped(&fuzzy_content, &fuzzy_old, 2);
                (fuzzy_old, fo, fc)
            } else {
                (base_old, match_offset, count)
            };

            if splice_count == 0 {
                return Ok(ToolResult {
                    content: format!("oldText not found in {}", path),
                    is_error: true,
                    image_blocks: vec![],
                });
            }
            if splice_count > 1 {
                return Ok(ToolResult {
                    content: format!(
                        "oldText matches {} times in {} — it must match exactly once to avoid \
                         ambiguous edits. Add more context to make it unique.",
                        splice_count, path
                    ),
                    is_error: true,
                    image_blocks: vec![],
                });
            }

            let offset = splice_offset.expect("count=1 guarantees Some(offset)");
            let normalised_new = base_normalise(new_text);

            // Splice into `content` (base-normalised), NOT fuzzy_content.
            // This preserves all non-edited content exactly as-is.
            let updated_lf = {
                let mut s =
                    String::with_capacity(content.len() - splice_old.len() + normalised_new.len());
                s.push_str(&content[..offset]);
                s.push_str(&normalised_new);
                s.push_str(&content[offset + splice_old.len()..]);
                s
            };

            // No-op detection: if the result is byte-identical, reject.
            if updated_lf == content {
                return Ok(ToolResult {
                    content: format!(
                        "No changes made to {}. The replacement produced identical content.",
                        path
                    ),
                    is_error: true,
                    image_blocks: vec![],
                });
            }

            // Produce diff before restoring line endings (work in LF space).
            let diff = make_edit_diff(path, &content, &updated_lf);

            // Restore original line endings and BOM before writing.
            let write_content = restore_file_format(&updated_lf, original_ending, has_bom);

            tokio::fs::write(&full_path, write_content.as_bytes())
                .await
                .map_err(|e| DomainError::Tool(format!("edit write failed: {}", e)))?;

            Ok(ToolResult {
                content: diff,
                is_error: false,
                image_blocks: vec![],
            })
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineEnding {
    Lf,
    Crlf,
}

/// Detect dominant line ending style via first occurrence heuristic.
fn detect_line_ending(s: &str) -> LineEnding {
    // Find position of first \r\n vs first lone \n.
    let crlf_pos = s.find("\r\n");
    let lf_pos = s.find('\n');
    match (crlf_pos, lf_pos) {
        (Some(cr), Some(lf)) if cr <= lf => LineEnding::Crlf,
        _ => LineEnding::Lf,
    }
}

/// Restore original line endings and re-prepend BOM if present.
fn restore_file_format(lf_content: &str, ending: LineEnding, has_bom: bool) -> String {
    let body = if ending == LineEnding::Crlf {
        lf_content.replace('\n', "\r\n")
    } else {
        lf_content.to_string()
    };
    if has_bom {
        format!("\u{FEFF}{}", body)
    } else {
        body
    }
}

/// Strip UTF-8 BOM and normalise CRLF → LF in a single pass.
fn base_normalise(s: &str) -> String {
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

/// Normalise text for fuzzy matching — mirrors Pi's `normalizeForFuzzyMatch`.
///
/// Applies on top of BOM stripping and CRLF normalisation:
/// - Trailing whitespace stripped per line
/// - Smart single quotes (U+2018–U+201B) → `'`
/// - Smart double quotes (U+201C–U+201F) → `"`
/// - Unicode dashes (U+2010–U+2015, U+2212) → `-`
/// - Special/non-breaking spaces → regular ASCII space
fn normalize_for_fuzzy_match(s: &str) -> String {
    // Start from base normalisation (BOM strip + CRLF→LF).
    let base = base_normalise(s);

    // Strip trailing whitespace per line, then apply char substitutions.
    base.lines()
        .map(|line| {
            let trimmed = line.trim_end();
            let mut out = String::with_capacity(trimmed.len());
            for c in trimmed.chars() {
                out.push(fuzzy_char(c));
            }
            out
        })
        .collect::<Vec<_>>()
        .join("\n")
        // Preserve trailing newline if original had one.
        + if base.ends_with('\n') { "\n" } else { "" }
}

/// Map a single character to its fuzzy-normalised equivalent.
#[inline]
fn fuzzy_char(c: char) -> char {
    match c {
        // Smart single quotes → straight single quote
        '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => '\'',
        // Smart double quotes → straight double quote
        '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => '"',
        // Unicode dashes / minus → ASCII hyphen-minus
        '\u{2010}'..='\u{2015}' | '\u{2212}' => '-',
        // Non-breaking and typographic spaces → regular space
        '\u{00A0}' | '\u{2002}'..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}' => ' ',
        other => other,
    }
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

/// Produce a unified-diff snippet using LCS-based line diffing via `similar`.
///
/// Context window is [`DIFF_CONTEXT_LINES`] lines on each side of each hunk.
/// If the diff exceeds [`DIFF_MAX_BYTES`] the full diff is omitted and only
/// the success message is returned.
fn make_edit_diff(path: &str, old_content: &str, new_content: &str) -> String {
    let diff = TextDiff::from_lines(old_content, new_content);
    let mut hunks_str = String::new();

    for group in diff.grouped_ops(DIFF_CONTEXT_LINES) {
        // Compute hunk header from the first op in the group.
        let first = group.first().unwrap();

        // old-file range (1-indexed)
        let old_start = first.old_range().start + 1;
        let old_len: usize = group.iter().map(|op| op.old_range().len()).sum();
        // new-file range (1-indexed)
        let new_start = first.new_range().start + 1;
        let new_len: usize = group.iter().map(|op| op.new_range().len()).sum();

        hunks_str.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            old_start, old_len, new_start, new_len
        ));

        for op in &group {
            for change in diff.iter_changes(op) {
                let prefix = match change.tag() {
                    ChangeTag::Delete => "-",
                    ChangeTag::Insert => "+",
                    ChangeTag::Equal => " ",
                };
                // `value()` includes the trailing newline; strip it and re-add
                // consistently so we control formatting.
                let line = change.value().trim_end_matches('\n');
                hunks_str.push_str(&format!("{}{}\n", prefix, line));
            }
        }
    }

    let diff_body = format!("Successfully edited {}\n\n```diff\n{}```", path, hunks_str);

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

    // --- Fuzzy content matching ---

    #[tokio::test]
    async fn test_edit_fuzzy_smart_single_quote() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("f.txt"), "it's a test").unwrap();
        let tool = EditTool::new(ws, sb);
        // oldText uses U+2019 RIGHT SINGLE QUOTATION MARK
        let result = tool
            .execute(r#"{"path":"f.txt","oldText":"it\u2019s a test","newText":"it's replaced"}"#)
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "smart quote should fuzzy match: {}",
            result.content
        );
        let content = std::fs::read_to_string(tmp.path().join("f.txt")).unwrap();
        assert_eq!(content, "it's replaced");
    }

    #[tokio::test]
    async fn test_edit_fuzzy_smart_double_quotes() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("f.txt"), "say \"hello\" now").unwrap();
        let tool = EditTool::new(ws, sb);
        // oldText uses U+201C/U+201D smart double quotes
        let result = tool
            .execute(
                "{\"path\":\"f.txt\",\"oldText\":\"say \\u201Chello\\u201D now\",\"newText\":\"say \\\"goodbye\\\" now\"}",
            )
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "smart double quotes should fuzzy match: {}",
            result.content
        );
        let content = std::fs::read_to_string(tmp.path().join("f.txt")).unwrap();
        assert!(
            content.contains("goodbye"),
            "expected replacement, got: {}",
            content
        );
    }

    #[tokio::test]
    async fn test_edit_fuzzy_preserves_non_edited_content() {
        // Fuzzy path must NOT fuzzy-rewrite the whole file; only the matched region changes.
        let (ws, sb, tmp) = test_tools();
        // Line 2 has smart quotes outside the edited region — must survive unchanged.
        let file = "say \"hello\" now\nline with \u{201C}preserved\u{201D} quotes\n";
        std::fs::write(tmp.path().join("f.txt"), file).unwrap();
        let tool = EditTool::new(ws, sb);
        let result = tool
            .execute(
                "{\"path\":\"f.txt\",\"oldText\":\"say \\u201Chello\\u201D now\",\"newText\":\"say hi now\"}",
            )
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "fuzzy match should succeed: {}",
            result.content
        );
        let content = std::fs::read_to_string(tmp.path().join("f.txt")).unwrap();
        assert!(
            content.contains('\u{201C}') && content.contains('\u{201D}'),
            "smart quotes outside edited region must be preserved, got: {:?}",
            content
        );
        assert!(
            content.contains("say hi now"),
            "replacement must appear: {:?}",
            content
        );
    }

    #[tokio::test]
    async fn test_edit_fuzzy_unicode_en_dash() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("f.txt"), "hello - world").unwrap();
        let tool = EditTool::new(ws, sb);
        // oldText uses U+2013 EN DASH
        let result = tool
            .execute(
                "{\"path\":\"f.txt\",\"oldText\":\"hello \\u2013 world\",\"newText\":\"replaced\"}",
            )
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "en-dash should fuzzy match: {}",
            result.content
        );
        let content = std::fs::read_to_string(tmp.path().join("f.txt")).unwrap();
        assert_eq!(content, "replaced");
    }

    #[tokio::test]
    async fn test_edit_fuzzy_trailing_whitespace() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("f.txt"), "hello\nworld").unwrap();
        let tool = EditTool::new(ws, sb);
        // oldText has trailing spaces on first line
        let result = tool
            .execute(r#"{"path":"f.txt","oldText":"hello   \nworld","newText":"replaced"}"#)
            .await
            .unwrap();
        assert!(
            !result.is_error,
            "trailing whitespace should fuzzy match: {}",
            result.content
        );
        let content = std::fs::read_to_string(tmp.path().join("f.txt")).unwrap();
        assert_eq!(content, "replaced");
    }

    // --- Line-ending preservation ---

    #[tokio::test]
    async fn test_edit_preserves_crlf_line_endings() {
        let (ws, sb, tmp) = test_tools();
        let crlf = "line1\r\nline2\r\nline3\r\n";
        std::fs::write(tmp.path().join("f.txt"), crlf).unwrap();
        let tool = EditTool::new(ws, sb);
        let result = tool
            .execute(r#"{"path":"f.txt","oldText":"line2","newText":"EDITED"}"#)
            .await
            .unwrap();
        assert!(!result.is_error, "edit should succeed: {}", result.content);
        let bytes = std::fs::read(tmp.path().join("f.txt")).unwrap();
        assert!(
            bytes.windows(2).any(|w| w == b"\r\n"),
            "CRLF line endings should be preserved in written file"
        );
        let text = String::from_utf8(bytes).unwrap();
        assert!(
            text.contains("EDITED"),
            "replacement should appear in output"
        );
    }

    // --- BOM preservation ---

    #[tokio::test]
    async fn test_edit_preserves_bom_on_write() {
        let (ws, sb, tmp) = test_tools();
        let bom_content = "\u{FEFF}hello world";
        std::fs::write(tmp.path().join("f.txt"), bom_content).unwrap();
        let tool = EditTool::new(ws, sb);
        let result = tool
            .execute(r#"{"path":"f.txt","oldText":"hello","newText":"hi"}"#)
            .await
            .unwrap();
        assert!(!result.is_error, "edit should succeed: {}", result.content);
        let bytes = std::fs::read(tmp.path().join("f.txt")).unwrap();
        assert!(
            bytes.starts_with(&[0xEF, 0xBB, 0xBF]),
            "UTF-8 BOM should be preserved on write, got: {:02X?}",
            &bytes[..bytes.len().min(6)]
        );
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("hi world"), "replacement should appear");
    }

    #[tokio::test]
    async fn test_edit_rejects_noop_replacement() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("f.txt"), "hello world").unwrap();
        let tool = EditTool::new(ws, sb);
        let result = tool
            .execute(r#"{"path":"f.txt","oldText":"hello world","newText":"hello world"}"#)
            .await
            .unwrap();
        assert!(result.is_error, "no-op replacement should be an error");
        assert!(
            result.content.contains("identical"),
            "error should mention 'identical': {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_edit_diff_context_4_lines() {
        let (ws, sb, tmp) = test_tools();
        // 10-line file; edit line 6 (f); context should include b,c,d,e (4 before)
        let content = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\n";
        std::fs::write(tmp.path().join("f.txt"), content).unwrap();
        let tool = EditTool::new(ws, sb);
        let result = tool
            .execute(r#"{"path":"f.txt","oldText":"f","newText":"F"}"#)
            .await
            .unwrap();
        assert!(!result.is_error, "edit should succeed: {}", result.content);
        // The diff should include 4 lines of context before the change
        assert!(
            result.content.contains("b"),
            "diff should contain 'b' as context"
        );
        assert!(
            result.content.contains("c"),
            "diff should contain 'c' as context"
        );
        assert!(
            result.content.contains("d"),
            "diff should contain 'd' as context"
        );
        assert!(
            result.content.contains("e"),
            "diff should contain 'e' as context"
        );
    }

    #[tokio::test]
    async fn test_edit_diff_uses_minus_plus_markers() {
        let (ws, sb, tmp) = test_tools();
        std::fs::write(tmp.path().join("f.txt"), "line1\nline2\nline3\n").unwrap();
        let tool = EditTool::new(ws, sb);
        let result = tool
            .execute(r#"{"path":"f.txt","oldText":"line2","newText":"CHANGED"}"#)
            .await
            .unwrap();
        assert!(!result.is_error, "edit should succeed: {}", result.content);
        assert!(
            result.content.contains("-line2"),
            "diff should contain '-line2'"
        );
        assert!(
            result.content.contains("+CHANGED"),
            "diff should contain '+CHANGED'"
        );
    }

    // --- normalize_for_fuzzy_match unit tests ---

    #[test]
    fn test_fuzzy_normalise_smart_single_quotes() {
        // U+2018 U+2019 U+201A U+201B → '
        for ch in ['\u{2018}', '\u{2019}', '\u{201A}', '\u{201B}'] {
            let input = format!("it{ch}s");
            let result = normalize_for_fuzzy_match(&input);
            assert_eq!(result, "it's", "char U+{:04X} should become '", ch as u32);
        }
    }

    #[test]
    fn test_fuzzy_normalise_smart_double_quotes() {
        // U+201C U+201D U+201E U+201F → "
        for ch in ['\u{201C}', '\u{201D}', '\u{201E}', '\u{201F}'] {
            let input = format!("{ch}hello{ch}");
            let result = normalize_for_fuzzy_match(&input);
            assert_eq!(
                result, "\"hello\"",
                "char U+{:04X} should become \"",
                ch as u32
            );
        }
    }

    #[test]
    fn test_fuzzy_normalise_unicode_dashes() {
        // U+2010–U+2015, U+2212 → -
        for ch in [
            '\u{2010}', '\u{2011}', '\u{2012}', '\u{2013}', '\u{2014}', '\u{2015}', '\u{2212}',
        ] {
            let input = format!("a{ch}b");
            let result = normalize_for_fuzzy_match(&input);
            assert_eq!(result, "a-b", "char U+{:04X} should become -", ch as u32);
        }
    }

    #[test]
    fn test_fuzzy_normalise_trailing_whitespace_per_line() {
        let input = "hello   \nworld  \n";
        let result = normalize_for_fuzzy_match(input);
        assert_eq!(result, "hello\nworld\n");
    }

    #[test]
    fn test_fuzzy_normalise_special_spaces() {
        // NBSP and ideographic space → regular space
        let input = "a\u{00A0}b\u{3000}c";
        let result = normalize_for_fuzzy_match(input);
        assert_eq!(result, "a b c");
    }

    #[test]
    fn test_fuzzy_normalise_strips_bom_and_crlf() {
        let input = "\u{FEFF}line1\r\nline2\r\n";
        let result = normalize_for_fuzzy_match(input);
        assert_eq!(result, "line1\nline2\n");
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

    #[tokio::test]
    async fn test_edit_empty_object_returns_actionable_error() {
        let (ws, sb, _tmp) = test_tools();
        let tool = EditTool::new(ws, sb);
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
    fn test_edit_description_includes_example() {
        let (ws, sb, _tmp) = test_tools();
        let tool = EditTool::new(ws, sb);
        let def = tool.definition();
        assert!(
            def.description.contains("Example"),
            "edit description should include Example, got: {}",
            def.description
        );
    }
}
