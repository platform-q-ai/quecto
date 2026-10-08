//! JSON delivery adapter for the typed find use case.
use crate::application::agent_turn::use_cases::find::{
    FindEntryKind, FindError, FindRequest, FindResult, FindUseCase,
};
use crate::application::tools::ports::Tool;
use crate::domain::error::DomainError;
use crate::domain::tool_policy::value_objects::tool::{ToolDefinition, ToolResult};
use std::future::Future;
use std::pin::Pin;

pub struct FindTool {
    use_case: FindUseCase,
}
impl FindTool {
    pub fn new(use_case: FindUseCase) -> Self {
        Self { use_case }
    }
}
impl Tool for FindTool {
    /// Reads only: calls may overlap (#2169).
    fn overlaps_safely(&self, _arguments: &str) -> bool {
        true
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "find".into(),
            description: "Find files and directories by glob pattern using fd. Requires fd on PATH. \
                          Returns newline-separated paths relative to the workspace, as grep \
                          prints them (absolute outside it), ready for read or edit; directories \
                          end in '/' (a symlink never does, whatever it points at). Respects .gitignore and lists hidden files, but skips \
                          VCS internals (.git, .hg, .svn, .jj) below the search path; pass one \
                          as path to search it. type \"d\" lists only directories (as does a \
                          pattern that ends in '/'), type \"f\" only files; a symlink counts \
                          as what it points at. Output capped at 1000 results or 50KB. \
                          Example: {\"pattern\": \"*.rs\"}"
                .into(),
            parameters_schema: r#"{
                "type": "object",
                "properties": {
                    "pattern": {"type":"string","description":"Glob pattern, e.g. '*.rs', '**/*.json', or 'src/*.rs' (path-segment patterns work)"},
                    "path":    {"type":"string","description":"Directory to search (defaults to '.'); a VCS directory such as '.git' is searched only when it is the path"},
                    "type":    {"type":"string","enum":["f","d"],"description":"f: files only; d: directories only (default: both)"},
                    "limit":   {"type":"number","description":"Maximum results (default 1000, max 100000)"}
                },
                "required": ["pattern"]
            }"#
            .into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args = serde_json::from_str::<serde_json::Value>(arguments);
        Box::pin(async move {
            let args = match args {
                Ok(args) => args,
                Err(error) => {
                    return Ok(result(
                        format!(
                            "invalid JSON arguments: {error}. Example: {{\"pattern\": \"*.rs\"}}"
                        ),
                        true,
                    ));
                }
            };
            let Some(pattern) = args.get("pattern").and_then(serde_json::Value::as_str) else {
                return Ok(result(
                    "missing 'pattern' argument. Example: {\"pattern\": \"*.rs\"}".into(),
                    true,
                ));
            };
            let (kind, limit) = match (entry_kind(args.get("type")), limit(args.get("limit"))) {
                (Ok(kind), Ok(limit)) => (kind, limit),
                (Err(message), _) | (_, Err(message)) => return Ok(result(message, true)),
            };
            let request = FindRequest {
                pattern: pattern.into(),
                path: args
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(".")
                    .into(),
                limit,
                kind,
            };
            match self.use_case.execute(request).await {
                Ok(found) => Ok(result(render(found), false)),
                Err(FindError::Security(message)) => Err(DomainError::Security(message)),
                Err(FindError::Spawn(message)) => Err(DomainError::Tool(message)),
                Err(FindError::Search(message) | FindError::Io(message)) => {
                    Ok(result(message, true))
                }
            }
        })
    }
}
/// The `type` argument, read against an allowlist (#2200): anything else is
/// refused rather than silently listing both kinds.
fn entry_kind(value: Option<&serde_json::Value>) -> Result<Option<FindEntryKind>, String> {
    match value {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(value) => match value.as_str() {
            Some("f" | "file") => Ok(Some(FindEntryKind::File)),
            Some("d" | "directory") => Ok(Some(FindEntryKind::Directory)),
            _ => Err(format!(
                "invalid 'type' {value}: use \"f\" (files only) or \"d\" (directories only), \
                 or omit it for both. Example: {{\"pattern\": \"*\", \"type\": \"d\"}}"
            )),
        },
    }
}

/// The `limit` argument: absent or null is the default; a number that
/// rounds to at least 1 is normalized by the use case; anything else is
/// refused, as ls and grep refuse it (#2200 reviews 2 and 3).
fn limit(value: Option<&serde_json::Value>) -> Result<Option<f64>, String> {
    match value.map(|value| (value, value.as_f64())) {
        None | Some((serde_json::Value::Null, _)) => Ok(None),
        // As grep's whole numbers: rounded, at least 1; above the maximum
        // is the maximum (the use case clamps).
        Some((_, Some(number))) if number.round() >= 1.0 => Ok(Some(number)),
        Some((value, _)) => Err(format!(
            "invalid 'limit' {value}: use a whole number of at least 1. \
             Example: {{\"pattern\": \"*.rs\", \"limit\": 100}}"
        )),
    }
}

/// What an empty search says: never a bare "nothing exists" (#2200).
/// The kind may have come from `type` or from a trailing '/', so the hint
/// names both; a skipped VCS directory is named only when there is one.
fn no_matches(kind: Option<FindEntryKind>, skipped_vcs_dir: Option<&str>) -> String {
    let said = match kind {
        None => {
            "No files found matching pattern (directories are listed too, ending in '/'; \
             type \"d\" lists only directories, type \"f\" only files"
        }
        Some(FindEntryKind::Directory) => {
            "No directories found matching pattern (only directories were searched, as \
             type \"d\" or a pattern ending in '/' asks; drop both to match files too"
        }
        Some(FindEntryKind::File) => {
            "No files found matching pattern (type \"f\" searched only files; omit it to \
             match directories too"
        }
    };
    match skipped_vcs_dir {
        Some(directory) => format!(
            "{said}; VCS metadata lives at {directory}, which is not searched unless passed as path)"
        ),
        None => format!("{said})"),
    }
}

fn result(content: String, is_error: bool) -> ToolResult {
    ToolResult {
        content,
        is_error,
        image_blocks: vec![],
        delivery_metadata: None,
    }
}
fn render(found: FindResult) -> String {
    const CAP: usize = crate::domain::constants::DEFAULT_OUTPUT_CAP_BYTES;
    let mut content = String::new();
    let mut byte_limited = false;
    for entry in found
        .output
        .entries
        .iter()
        .filter(|entry| matches!(entry.as_bytes(), [_, ..]))
    {
        let separator = if content.is_empty() { 0 } else { 1 };
        if entry.len() <= CAP.saturating_sub(content.len() + separator) {
            append_line(&mut content, entry);
        } else {
            byte_limited = true;
            break;
        }
    }
    assert!(content.len() <= CAP, "find payload cap invariant");
    if byte_limited {
        append_line(&mut content, "[50KB limit reached]");
    }
    // Said even under the byte cap: what is shown is still drawn from an
    // arbitrary subset (#2176 review).
    // With nothing shown, a bigger limit is no advice: the incompleteness
    // note says why (#2200 review 3).
    if let (true, false) = (
        found.output.result_limit_reached,
        found.output.entries.is_empty(),
    ) {
        append_line(
            &mut content,
            &format!(
                "[Results limit reached: the search stops at its limit, so this listing is an arbitrary subset of the matches, not the first. Use limit={} for more, or refine pattern]",
                found.limit.saturating_mul(2)
            ),
        );
    }
    if found.output.incomplete {
        append_line(
            &mut content,
            "[Search incomplete; refine pattern or search a narrower path]",
        );
        if let Some(diagnostic) = found
            .output
            .diagnostic
            .as_deref()
            .filter(|text| matches!(text.as_bytes(), [_, ..]))
        {
            append_line(&mut content, diagnostic);
        }
    }
    if content.is_empty() {
        no_matches(found.kind, found.output.skipped_vcs_dir.as_deref())
    } else {
        content
    }
}

fn append_line(content: &mut String, line: &str) {
    if content.is_empty() {
        content.push_str(line);
    } else {
        content.push('\n');
        content.push_str(line);
    }
}
#[cfg(test)]
#[path = "find_tests.rs"]
mod tests;
