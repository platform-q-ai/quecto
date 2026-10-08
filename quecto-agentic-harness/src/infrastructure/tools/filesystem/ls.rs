// LsTool — tool name: "ls"

use std::collections::BinaryHeap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::tool_policy::value_objects::tool::{ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;

use super::fs_failure::{Access, explain};
use super::resolve_and_validate;

/// Default maximum number of directory entries to show (Quecto compatibility: 500).
const LS_DEFAULT_LIMIT: usize = 500;
/// Maximum allowed limit (prevents abuse).
const LS_MAX_LIMIT: usize = 5000;
/// Maximum offset: the window kept while reading is offset + limit entries,
/// so this bounds memory on a huge directory (#2188).
const LS_MAX_OFFSET: usize = 100_000;
/// Maximum output bytes before truncating.
const LS_MAX_BYTES: usize = crate::domain::constants::DEFAULT_OUTPUT_CAP_BYTES;

pub struct LsTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
}

impl LsTool {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self { workspace, sandbox }
    }
}

impl Tool for LsTool {
    /// Reads only: calls may overlap (#2169).
    fn overlaps_safely(&self, _arguments: &str) -> bool {
        true
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "ls".into(),
            description: format!(
                "List directory contents. Defaults to the current working directory \
                 when path is omitted. Entries are sorted case-insensitively; directories \
                 are suffixed with '/'. Output capped at {} entries or 50KB; a longer \
                 listing is the sorted start of the directory, and offset shows the rest. \
                 The whole directory is read each call, so a huge one is slow: prefer a \
                 narrower path or find with a pattern there. \
                 Example: {{\"path\": \"src\"}}",
                LS_DEFAULT_LIMIT
            )
            .into(),
            parameters_schema: format!(
                r#"{{
                "type": "object",
                "properties": {{
                    "path":   {{"type":"string","description":"Directory path to list (relative or absolute, defaults to '.')"}},
                    "limit":  {{"type":"number","description":"Maximum number of entries to return (default {}, max {})"}},
                    "offset": {{"type":"number","description":"Sorted entries to skip before listing (default 0, max {}): pages through a large directory"}}
                }}
            }}"#,
                LS_DEFAULT_LIMIT, LS_MAX_LIMIT, LS_MAX_OFFSET
            )
            .into(),
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
            // LLM-addressable: malformed JSON → Ok(is_error=true). Tool contract.
            let args: serde_json::Value = match serde_json::from_str(&args_str) {
                Ok(v) => v,
                Err(e) => {
                    return Ok(listing_result(
                        format!("invalid JSON arguments: {e}. Example: {{\"path\": \".\"}}"),
                        true,
                    ));
                }
            };

            let path = args["path"].as_str().unwrap_or(".");
            let full_path = resolve_and_validate(&workspace, &sandbox, path)?;
            // Integers and floats are both numbers (see `number`).
            let (limit, offset) = match (limit(&args["limit"]), offset(&args["offset"])) {
                (Ok(limit), Ok(offset)) => (limit, offset),
                (Err(message), _) | (_, Err(message)) => {
                    return Ok(listing_result(message, true));
                }
            };
            assert!((1..=LS_MAX_LIMIT).contains(&limit) && offset <= LS_MAX_OFFSET);

            let mut entries_raw = match tokio::fs::read_dir(&full_path).await {
                Ok(entries) => entries,
                Err(error) => {
                    let reason = explain(Access::List, &full_path, path, &error).await;
                    return Ok(listing_result(reason, true));
                }
            };
            // The whole directory is read so the listing is its true sorted
            // start (#2188); only the first offset + limit are kept. There
            // is no deadline: the tool port carries no cancellation token,
            // and a dropped call stops at the next entry's await.
            let mut window = SortedWindow::new(offset + limit);
            loop {
                let entry = match entries_raw.next_entry().await {
                    Ok(Some(entry)) => entry,
                    Ok(None) => break,
                    Err(error) => {
                        let reason = explain(Access::List, &full_path, path, &error).await;
                        return Ok(listing_result(reason, true));
                    }
                };
                let name = entry.file_name().to_string_lossy().to_string();
                let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);
                window.offer(if is_dir { format!("{name}/") } else { name });
            }
            Ok(listing_result(render_listing(window, offset, limit), false))
        })
    }
}

/// The smallest `capacity` names under the listing's sort key, and how many
/// were offered. Memory is O(capacity) however large the directory.
struct SortedWindow {
    capacity: usize,
    kept: BinaryHeap<(String, String)>,
    total: usize,
}

impl SortedWindow {
    fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "a listing window keeps at least one entry");
        Self {
            capacity,
            kept: BinaryHeap::new(),
            total: 0,
        }
    }

    /// Case-insensitive, with the exact name breaking ties so the order
    /// never depends on the order entries arrive in.
    fn offer(&mut self, name: String) {
        self.total += 1;
        let key = (name.to_lowercase(), name);
        match self.kept.peek() {
            Some(largest) if self.kept.len() == self.capacity => {
                if key < *largest {
                    self.kept.pop();
                    self.kept.push(key);
                }
            }
            _ => self.kept.push(key),
        }
        assert!(self.kept.len() <= self.capacity, "listing window bound");
    }

    fn total(&self) -> usize {
        self.total
    }

    fn into_sorted(self) -> Vec<String> {
        self.kept
            .into_sorted_vec()
            .into_iter()
            .map(|(_, name)| name)
            .collect()
    }
}

/// An optional numeric argument, as grep reads whole numbers: absent or
/// null is `None`; a number is rounded and must be at least `min` (a
/// saturating cast bounds a huge one); anything else is refused with an
/// example, never silently replaced.
fn number(name: &str, value: &serde_json::Value, min: usize) -> Result<Option<usize>, String> {
    let refused = |value: &serde_json::Value| {
        format!(
            "invalid '{name}' {value}: use a whole number of at least {min}. \
             Example: {{\"path\": \"src\", \"limit\": 100, \"offset\": 100}}"
        )
    };
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Number(number) => match number.as_f64().map(f64::round) {
            Some(rounded) if rounded >= min as f64 => Ok(Some(rounded as usize)),
            Some(_) | None => Err(refused(value)),
        },
        other => Err(refused(other)),
    }
}

/// The limit: at least 1; above `LS_MAX_LIMIT` is the maximum, as
/// documented.
fn limit(value: &serde_json::Value) -> Result<usize, String> {
    Ok(number("limit", value, 1)?.map_or(LS_DEFAULT_LIMIT, |limit| limit.min(LS_MAX_LIMIT)))
}

/// The offset, at most `LS_MAX_OFFSET`: the entries kept while reading are
/// offset + limit, so a larger one is refused rather than clamped to a page
/// that was not asked for (#2188 review 2). The refusal echoes what was
/// sent, not a saturated number.
fn offset(value: &serde_json::Value) -> Result<usize, String> {
    match number("offset", value, 0)? {
        None => Ok(0),
        Some(offset) if offset <= LS_MAX_OFFSET => Ok(offset),
        Some(_) => Err(format!(
            "invalid 'offset' {value}: at most {LS_MAX_OFFSET}. For later entries of a huge \
             directory, use find with a pattern and this path, e.g. \
             {{\"pattern\": \"f2*\", \"path\": \"dir\"}}"
        )),
    }
}

/// The page at `offset`, whole entries within the byte cap, and a note that
/// says what was left out and how to see it.
fn render_listing(window: SortedWindow, offset: usize, limit: usize) -> String {
    let total = window.total();
    if total == 0 {
        // Quecto compatibility: empty directory message.
        return "(empty directory)".to_string();
    }
    let sorted = window.into_sorted();
    let page = sorted.get(offset..).unwrap_or_default();
    if page.is_empty() {
        return format!("(no entries at offset {offset}; the directory has {total} entries)");
    }
    assert!(page.len() <= limit, "a page holds at most limit entries");
    // The whole response stays within the cap (#2188 PR review): when a
    // note will follow, the entries leave room for the longest it can be.
    let whole = page.iter().map(|name| name.len() + 1).sum::<usize>() - 1;
    let budget = match (whole <= LS_MAX_BYTES, offset + page.len() < total) {
        (true, false) => LS_MAX_BYTES,
        (true, true) | (false, _) => LS_MAX_BYTES - note_reserve(offset, total, limit),
    };
    let mut output = String::new();
    let mut shown = 0;
    for name in page {
        let separator = usize::from(shown > 0);
        if output.len() + separator + name.len() <= budget {
            if separator == 1 {
                output.push('\n');
            }
            output.push_str(name);
            shown += 1;
        } else {
            break;
        }
    }
    // A name is at most a few hundred bytes: the first always fits.
    assert!(shown > 0, "a non-empty page shows at least one entry");
    let next = offset + shown;
    if next < total {
        // shown > 0: the page's entries are above the note.
        output.push('\n');
        output.push_str(&note(offset, next, total, shown < page.len(), limit));
    }
    assert!(output.len() <= LS_MAX_BYTES, "ls response cap invariant");
    output
}

/// What a page left out, and how to see it.
fn note(offset: usize, next: usize, total: usize, capped: bool, limit: usize) -> String {
    composed_note(
        offset,
        next,
        total,
        &reason(capped, limit),
        &continuation(next),
    )
}

fn composed_note(
    offset: usize,
    next: usize,
    total: usize,
    reason: &str,
    continuation: &str,
) -> String {
    format!(
        "[Entries {}-{next} of {total} shown (sorted case-insensitively; {reason}). {continuation}]",
        offset + 1
    )
}

fn reason(capped: bool, limit: usize) -> String {
    match capped {
        true => "50KB output cap reached".to_string(),
        false => format!("limit {limit} reached"),
    }
}

fn continuation(next: usize) -> String {
    match next <= LS_MAX_OFFSET {
        true => format!(
            "Next: offset={next}. Or raise limit (max {LS_MAX_LIMIT}), or list a more specific path"
        ),
        false => format!(
            "offset stops at {LS_MAX_OFFSET}: use find with a pattern and this path for later entries"
        ),
    }
}

/// Room for the newline and the longest note this page can end with: the
/// next entry is at most `total` (and at most `LS_MAX_OFFSET` where it is
/// named as an offset), and every reason and continuation is measured.
fn note_reserve(offset: usize, total: usize, limit: usize) -> usize {
    let continuations = [
        continuation(total.min(LS_MAX_OFFSET)),
        continuation(LS_MAX_OFFSET + 1),
    ];
    let longest = [reason(true, limit), reason(false, limit)]
        .iter()
        .flat_map(|reason| {
            continuations
                .iter()
                .map(|continuation| composed_note(offset, total, total, reason, continuation).len())
        })
        .max()
        .expect("notes were measured");
    1 + longest
}

fn listing_result(content: String, is_error: bool) -> ToolResult {
    ToolResult {
        content,
        is_error,
        image_blocks: vec![],
        delivery_metadata: None,
    }
}

#[cfg(test)]
#[path = "ls_tests.rs"]
mod tests;
