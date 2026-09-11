//! JSON delivery adapter for the typed find use case.
use crate::application::agent_turn::use_cases::find::{
    FindError, FindRequest, FindResult, FindUseCase,
};
use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
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
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "find".into(),
            description: "Find files by glob pattern using fd. Requires fd on PATH. \
                          Returns newline-separated relative paths. Respects .gitignore. \
                          Output capped at 1000 results or 50KB. \
                          Example: {\"pattern\": \"*.rs\"}"
                .into(),
            parameters_schema: r#"{
                "type": "object",
                "properties": {
                    "pattern": {"type":"string","description":"Glob pattern, e.g. '*.rs', '**/*.json', or 'src/*.rs' (path-segment patterns work)"},
                    "path":    {"type":"string","description":"Directory to search (defaults to '.')"},
                    "limit":   {"type":"number","description":"Maximum results (default 1000)"}
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
            let request = FindRequest {
                pattern: pattern.into(),
                path: args
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(".")
                    .into(),
                limit: args.get("limit").and_then(serde_json::Value::as_f64),
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
    } else if found.output.result_limit_reached {
        append_line(
            &mut content,
            &format!(
                "[{} results limit reached. Use limit={} for more, or refine pattern]",
                found.limit,
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
        "No files found matching pattern".into()
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
