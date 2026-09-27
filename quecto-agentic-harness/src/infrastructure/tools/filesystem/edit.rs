// EditTool — tool name: "edit"
// Two-stage exact→fuzzy matching, no-op detection, LCS-based unified diff
// (edit_diff.rs). The edit is spliced into the file's own bytes: nothing
// outside the matched span changes, line endings included (#2242).

use std::borrow::Cow;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::tool::{ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;

use super::edit_bytes::{line_view, span_line_ending, splice, view_offset, with_span_endings};
use super::edit_diff::{Change, make_edit_diff};
use super::edit_match::{Location, Unmappable, base_mapped, locate};
use super::edit_refusal::{FileLines, ambiguous, load_text, not_found, write_refusal};
use super::fs_failure::refused;
use super::resolve_and_validate;

pub(super) const MAX_EDIT_FILE_BYTES: u64 = 1024 * 1024;
// The fuzzy matcher's offset map stores `u32` offsets (edit_match.rs).
const _: () = assert!(MAX_EDIT_FILE_BYTES <= u32::MAX as u64);

const EDIT_EXAMPLE: &str = "{\"path\": \"file.txt\", \"oldText\": \"old\", \"newText\": \"new\"}";

fn missing_edit_arg(param: &str) -> ToolResult {
    ToolResult {
        content: format!("missing '{}' argument. Example: {}", param, EDIT_EXAMPLE),
        is_error: true,
        image_blocks: vec![],
        delivery_metadata: None,
    }
}

/// #2191: a match that cannot be placed exactly in the file is never written.
fn unmappable_match(path: &str) -> ToolResult {
    refused(format!(
        "edit refused: the match could not be mapped back to the text of {} exactly, \
         so nothing was written. Copy oldText exactly from the file and retry.",
        path
    ))
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
            let raw = match load_text(&full_path, path).await {
                Ok(raw) => raw,
                Err(refusal) => return Ok(refusal),
            };

            // Match on the normalised text (BOM stripped, every line break
            // `\n`), exact first, then the fuzzy fallback; the offset map
            // places the match in the file's own bytes (#2242).
            let Ok(mapped) = base_mapped(&raw) else {
                return Ok(unmappable_match(path));
            };
            let content = mapped.text.as_str();
            let base_old = base_normalise(old_text);

            // The range is proven to be on character boundaries of `content`.
            let range = match locate(content, &base_old) {
                Ok(Location::Unique(range)) => range,
                Ok(Location::NotFound) => {
                    let lines = FileLines::of(&raw);
                    return Ok(not_found(content, &lines, &base_old, path));
                }
                Ok(Location::Ambiguous(matches)) => {
                    let lines = FileLines::of(&raw);
                    return Ok(ambiguous(&lines, path, &matches));
                }
                Err(Unmappable) => return Ok(unmappable_match(path)),
            };

            // newText's line breaks take the endings of the lines they
            // replace; every byte outside the matched span is kept as it is.
            let Some(raw_range) = mapped.original_range(&raw, range) else {
                return Ok(unmappable_match(path));
            };
            let normalised_new = base_normalise(new_text);
            let replaced = raw.get(raw_range.clone()).unwrap_or_default();
            let beyond = span_line_ending(&raw, raw_range.clone());
            let inserted = with_span_endings(&normalised_new, replaced, beyond);
            let Some(updated) = splice(&raw, raw_range.clone(), &inserted) else {
                return Ok(unmappable_match(path));
            };

            // No-op: the same bytes, or the same lines as read shows them
            // (only a line break's form would change).
            let (old_view, new_view) = (line_view(&raw), line_view(&updated));
            if updated == raw || new_view == old_view {
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

            // The diff shows the file's lines as read does: its own line
            // breaks number them.
            let view_start = view_offset(&raw, raw_range.start);
            let view_range = view_start..view_offset(&raw, raw_range.end);
            let change = Change::of_splice(&old_view, &new_view, view_range, normalised_new.len());
            let diff = make_edit_diff(path, &old_view, &new_view, &change);

            if let Err(error) = tokio::fs::write(&full_path, updated.as_bytes()).await {
                return Ok(write_refusal(path, &error));
            }

            Ok(ToolResult {
                content: diff,
                is_error: false,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

/// Strip UTF-8 BOM and normalise CRLF → LF in a single pass.
pub(super) fn base_normalise(s: &str) -> Cow<'_, str> {
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

#[cfg(test)]
#[path = "edit_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "edit_fuzzy_file_tests.rs"]
mod fuzzy_file_tests;

#[cfg(test)]
#[path = "edit_bytes_tests.rs"]
mod bytes_tests;
