// EditTool — tool name: "edit"
// Two-stage exact→fuzzy matching, CRLF/BOM preservation, no-op detection,
// LCS-based unified diff via the `similar` crate.

use std::borrow::Cow;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use similar::{ChangeTag, TextDiff};

use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::tool::{ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;

use super::edit_match::{Location, Unmappable, locate};
use super::resolve_and_validate;

const MAX_EDIT_FILE_BYTES: u64 = 1024 * 1024;
// The fuzzy matcher's offset map stores `u32` offsets (edit_match.rs).
const _: () = assert!(MAX_EDIT_FILE_BYTES <= u32::MAX as u64);
const DIFF_MAX_BYTES: usize = 4096;
const DIFF_CONTEXT_LINES: usize = 4;

const EDIT_EXAMPLE: &str = "{\"path\": \"file.txt\", \"oldText\": \"old\", \"newText\": \"new\"}";

fn missing_edit_arg(param: &str) -> ToolResult {
    ToolResult {
        content: format!("missing '{}' argument. Example: {}", param, EDIT_EXAMPLE),
        is_error: true,
        image_blocks: vec![],
        delivery_metadata: None,
    }
}

fn edit_refused(content: String) -> ToolResult {
    ToolResult {
        content,
        is_error: true,
        image_blocks: vec![],
        delivery_metadata: None,
    }
}

/// #2191: a match that cannot be placed exactly in the file is never written.
fn unmappable_match(path: &str) -> ToolResult {
    edit_refused(format!(
        "edit refused: the match could not be mapped back to the text of {} exactly, \
         so nothing was written. Copy oldText exactly from the file and retry.",
        path
    ))
}

/// Replace `range` of `content` with `new`, or `None` when the range is not
/// on character boundaries of `content`.
fn splice(content: &str, range: std::ops::Range<usize>, new: &str) -> Option<String> {
    let range = (range.start <= range.end).then_some(range)?;
    let before = content.get(..range.start)?;
    let after = content.get(range.end..)?;
    let mut out = String::with_capacity(before.len() + new.len() + after.len());
    out.push_str(before);
    out.push_str(new);
    out.push_str(after);
    Some(out)
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
                        delivery_metadata: None,
                    });
                }
            };
            let Some(path) = args["path"].as_str() else {
                return Ok(missing_edit_arg("path"));
            };
            // Accept "oldText" (tool name) or legacy "old"
            let Some(old_text) = args["oldText"].as_str().or_else(|| args["old"].as_str()) else {
                return Ok(missing_edit_arg("oldText"));
            };
            // Accept "newText" (tool name) or legacy "new"
            let Some(new_text) = args["newText"].as_str().or_else(|| args["new"].as_str()) else {
                return Ok(missing_edit_arg("newText"));
            };

            // Empty text matches nothing useful: say so, not "not found" (#2166).
            if base_normalise(old_text).is_empty() {
                return Ok(ToolResult {
                    content: "oldText must not be empty: give the exact text to replace, \
                              with enough surrounding lines to match once"
                        .into(),
                    is_error: true,
                    image_blocks: vec![],
                    delivery_metadata: None,
                });
            }

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

            // Exact match first, then the fuzzy fallback, on BOM-stripped /
            // CRLF-normalised text.
            let content = base_normalise(&raw);
            let base_old = base_normalise(old_text);

            // `content` is what we splice into — the base-normalised file. The
            // range is proven to be on character boundaries of `content`.
            let range = match locate(&content, &base_old) {
                Ok(Location::Unique(range)) => range,
                Ok(Location::NotFound) => {
                    return Ok(edit_refused(format!("oldText not found in {}", path)));
                }
                Ok(Location::Ambiguous(count)) => {
                    return Ok(edit_refused(format!(
                        "oldText matches {} times in {} — it must match exactly once to avoid \
                         ambiguous edits. Add more context to make it unique.",
                        count, path
                    )));
                }
                Err(Unmappable) => return Ok(unmappable_match(path)),
            };

            let normalised_new = base_normalise(new_text);
            let Some(updated_lf) = splice(&content, range, &normalised_new) else {
                return Ok(unmappable_match(path));
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
                    delivery_metadata: None,
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
                delivery_metadata: None,
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
fn restore_file_format(lf_content: &str, ending: LineEnding, has_bom: bool) -> Cow<'_, str> {
    let body = if ending == LineEnding::Crlf {
        Cow::Owned(lf_content.replace('\n', "\r\n"))
    } else {
        Cow::Borrowed(lf_content)
    };
    if has_bom {
        Cow::Owned(format!("\u{FEFF}{}", body))
    } else {
        body
    }
}

/// Strip UTF-8 BOM and normalise CRLF → LF in a single pass.
fn base_normalise(s: &str) -> Cow<'_, str> {
    let s = s.strip_prefix('\u{FEFF}').unwrap_or(s);
    if !s.contains('\r') {
        return Cow::Borrowed(s);
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
    Cow::Owned(out)
}

/// Produce a unified-diff snippet using LCS-based line diffing via `similar`.
///
/// Context window is [`DIFF_CONTEXT_LINES`] lines on each side of each hunk.
/// If the rendered diff exceeds [`DIFF_MAX_BYTES`], a bounded prefix of concrete
/// diff lines is returned with a truncation notice instead of dropping all diff
/// context.
/// Generate a Quecto-style diff with per-line numbers and context ellipsis.
///
/// Output format (matching Quecto's `generateDiffString`):
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

    let ops = diff.grouped_ops(DIFF_CONTEXT_LINES);
    let total_hunks = ops.len();
    let mut changed_lines_total = 0usize;
    let mut output = Vec::new();
    let mut hunk_end_line_indexes = Vec::new();

    for (group_idx, group) in ops.iter().enumerate() {
        for op in group {
            for change in diff.iter_changes(op) {
                let line = change.value().trim_end_matches('\n');
                match change.tag() {
                    ChangeTag::Delete => {
                        changed_lines_total += 1;
                        let n = change.old_index().unwrap_or(0) + 1;
                        output.push(format!("-{:>width$} {}", n, line, width = num_width));
                    }
                    ChangeTag::Insert => {
                        changed_lines_total += 1;
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
        hunk_end_line_indexes.push(output.len());
        // Add ellipsis between hunks (not after the last one).
        if group_idx + 1 < total_hunks {
            output.push(format!(" {:>width$} ...", "", width = num_width));
        }
    }

    render_bounded_edit_diff(
        path,
        output,
        hunk_end_line_indexes,
        total_hunks,
        changed_lines_total,
    )
}

fn render_bounded_edit_diff(
    path: &str,
    diff_lines: Vec<String>,
    hunk_end_line_indexes: Vec<usize>,
    total_hunks: usize,
    changed_lines_total: usize,
) -> String {
    let full = format!("Successfully edited {}\n\n{}", path, diff_lines.join("\n"));
    if full.len() <= DIFF_MAX_BYTES {
        return full;
    }

    let mut shown = Vec::new();
    for line in &diff_lines {
        shown.push(line.clone());
        let hunks_shown = count_complete_hunks_shown(shown.len(), &hunk_end_line_indexes);
        let notice = diff_truncated_notice(hunks_shown, total_hunks, changed_lines_total);
        let prefix = truncated_success_prefix(path, notice.len(), 1);
        let candidate = format!("{}{}\n{}", prefix, shown.join("\n"), notice);
        if candidate.len() > DIFF_MAX_BYTES {
            shown.pop();
            break;
        }
    }

    let mut hunks_shown = count_complete_hunks_shown(shown.len(), &hunk_end_line_indexes);
    let notice = diff_truncated_notice(hunks_shown, total_hunks, changed_lines_total);
    if shown.is_empty() {
        shown = truncated_first_change_pair(path, &diff_lines, notice.len());
        hunks_shown = count_complete_hunks_shown(shown.len(), &hunk_end_line_indexes);
    }
    let notice = diff_truncated_notice(hunks_shown, total_hunks, changed_lines_total);
    let prefix = truncated_success_prefix(path, notice.len(), shown.join("\n").len());
    format!("{}{}\n{}", prefix, shown.join("\n"), notice)
}

fn truncated_first_change_pair(
    path: &str,
    diff_lines: &[String],
    notice_len: usize,
) -> Vec<String> {
    let Some(first_line) = diff_lines.first() else {
        return Vec::new();
    };
    let second_change_line = diff_lines
        .iter()
        .skip(1)
        .find(|line| line.starts_with('+') || line.starts_with('-'));
    let reserved_second_len = second_change_line
        .map(|line| line.len().min(8))
        .unwrap_or_default();
    let reserved_diff_len =
        first_line.len().min(16) + reserved_second_len + usize::from(second_change_line.is_some());
    let prefix = truncated_success_prefix(path, notice_len, reserved_diff_len);
    let mut budget = DIFF_MAX_BYTES.saturating_sub(prefix.len() + notice_len + 1);

    let mut shown = Vec::new();
    let first_budget =
        budget.saturating_sub(reserved_second_len + usize::from(second_change_line.is_some()));
    shown.push(truncate_to_byte_budget(first_line, first_budget));
    budget = budget.saturating_sub(shown[0].len());

    if let Some(second_line) = second_change_line {
        budget = budget.saturating_sub(1);
        let second = truncate_to_byte_budget(second_line, budget);
        if !second.is_empty() {
            shown.push(second);
        }
    }

    shown
}

fn truncated_success_prefix(path: &str, notice_len: usize, diff_len: usize) -> String {
    let boilerplate_len = "Successfully edited \n\n\n".len();
    let path_budget = DIFF_MAX_BYTES.saturating_sub(boilerplate_len + notice_len + diff_len);
    format!(
        "Successfully edited {}\n\n",
        truncate_to_byte_budget(path, path_budget)
    )
}

fn count_complete_hunks_shown(shown_lines: usize, hunk_end_line_indexes: &[usize]) -> usize {
    hunk_end_line_indexes
        .iter()
        .take_while(|end| shown_lines >= **end)
        .count()
}

fn truncate_to_byte_budget(line: &str, budget: usize) -> String {
    line.char_indices()
        .map(|(idx, _)| idx)
        .chain(std::iter::once(line.len()))
        .take_while(|idx| *idx <= budget)
        .last()
        .and_then(|end| line.get(..end))
        .map(str::to_string)
        .unwrap_or_default()
}

fn diff_truncated_notice(
    hunks_shown: usize,
    total_hunks: usize,
    changed_lines_total: usize,
) -> String {
    format!(
        "[diff truncated: {} of {} hunks shown, {} lines changed total]",
        hunks_shown, total_hunks, changed_lines_total
    )
}

#[cfg(test)]
#[path = "edit_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "edit_fuzzy_file_tests.rs"]
mod fuzzy_file_tests;
