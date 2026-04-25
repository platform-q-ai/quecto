// EditTool — Pi name: "edit"
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

const EDIT_EXAMPLE: &str = "{\"path\": \"file.txt\", \"oldText\": \"old\", \"newText\": \"new\"}";

fn missing_edit_arg(param: &str) -> ToolResult {
    ToolResult {
        content: format!("missing '{}' argument. Example: {}", param, EDIT_EXAMPLE),
        is_error: true,
        image_blocks: vec![],
    }
}

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
            name: "edit".into(),
            description: "Edit a file by replacing exact text. The oldText must match exactly \
                          (including whitespace). Use this for precise, surgical edits. \
                          Example: {\"path\": \"file.txt\", \"oldText\": \"old\", \"newText\": \"new\"}"
                .into(),
            parameters_schema: r#"{"type":"object","properties":{
                "path":{"type":"string","description":"Path to the file to edit (relative or absolute)"},
                "oldText":{"type":"string","description":"Exact text to find and replace (must match exactly)"},
                "newText":{"type":"string","description":"New text to replace the old text with"}
            },"required":["path","oldText","newText"]}"#
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

        Box::pin(async move {
            // LLM-addressable: malformed JSON → Ok(is_error=true). Tool contract.
            let args = match args {
                Ok(v) => v,
                Err(e) => {
                    return Ok(ToolResult {
                        content: format!(
                            "invalid JSON arguments: {e}. Example: {{\"path\": \"f.txt\", \"oldText\": \"a\", \"newText\": \"b\"}}"
                        ),
                        is_error: true,
                        image_blocks: vec![],
                    });
                }
            };
            let Some(path) = args["path"].as_str() else {
                return Ok(missing_edit_arg("path"));
            };
            // Accept "oldText" (Pi name) or legacy "old"
            let Some(old_text) = args["oldText"].as_str().or_else(|| args["old"].as_str()) else {
                return Ok(missing_edit_arg("oldText"));
            };
            // Accept "newText" (Pi name) or legacy "new"
            let Some(new_text) = args["newText"].as_str().or_else(|| args["new"].as_str()) else {
                return Ok(missing_edit_arg("newText"));
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
/// Generate a Pi-style diff with per-line numbers and context ellipsis.
///
/// Output format (matching Pi's `generateDiffString`):
/// ```text
///  11 context line
/// -12 removed line
/// +12 added line
///     ...
/// ```
fn make_edit_diff(path: &str, old_content: &str, new_content: &str) -> String {
    let diff = TextDiff::from_lines(old_content, new_content);
    let max_line = old_content.lines().count().max(new_content.lines().count());
    let num_width = if max_line == 0 {
        1
    } else {
        max_line.to_string().len()
    };

    let mut output = Vec::new();

    let ops = diff.grouped_ops(DIFF_CONTEXT_LINES);
    for (group_idx, group) in ops.iter().enumerate() {
        for op in group {
            for change in diff.iter_changes(op) {
                let line = change.value().trim_end_matches('\n');
                match change.tag() {
                    ChangeTag::Delete => {
                        let n = change.old_index().unwrap_or(0) + 1;
                        output.push(format!("-{:>width$} {}", n, line, width = num_width));
                    }
                    ChangeTag::Insert => {
                        let n = change.new_index().unwrap_or(0) + 1;
                        output.push(format!("+{:>width$} {}", n, line, width = num_width));
                    }
                    ChangeTag::Equal => {
                        let n = change.old_index().unwrap_or(0) + 1;
                        output.push(format!(" {:>width$} {}", n, line, width = num_width));
                    }
                }
            }
        }
        // Add ellipsis between hunks (not after the last one).
        if group_idx + 1 < ops.len() {
            output.push(format!(" {:>width$} ...", "", width = num_width));
        }
    }

    let diff_body = format!("Successfully edited {}\n\n{}", path, output.join("\n"));

    if diff_body.len() > DIFF_MAX_BYTES {
        format!("Successfully edited {}", path)
    } else {
        diff_body
    }
}

#[cfg(test)]
#[path = "edit_tests.rs"]
mod tests;
