// DocsTool: serves quecto's own capability docs, embedded in the binary.
//
// Agent-facing guides live under `docs/docs-tool-embeds/` and are baked in at
// compile time via `include_str!` so they are reachable from ANY working
// directory — reading `docs/*.md` from disk breaks whenever quecto runs
// outside its own checkout (paths resolve relative to the agent's CWD).

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use std::future::Future;
use std::pin::Pin;

/// The embedded capability docs, keyed by short name (no path prefix, no
/// `.md` suffix). These are the agent-facing guides under
/// `docs/docs-tool-embeds/`; human-only docs (README, getting-started, PRDs,
/// ADRs, etc.) are intentionally not embedded.
///
/// `include_str!` paths are relative to this source file
/// (`src/infrastructure/tools/`), so `../../../docs/docs-tool-embeds/` is the
/// package embed folder. A renamed/removed doc fails the build — the embed
/// cannot silently drift.
const EMBEDDED_DOCS: &[(&str, &str)] = &[
    (
        "subagents",
        include_str!("../../../docs/docs-tool-embeds/subagents.md"),
    ),
    (
        "workflow",
        include_str!("../../../docs/docs-tool-embeds/workflow.md"),
    ),
    (
        "extensions",
        include_str!("../../../docs/docs-tool-embeds/extensions.md"),
    ),
    (
        "sessions",
        include_str!("../../../docs/docs-tool-embeds/sessions.md"),
    ),
    (
        "uds-protocol",
        include_str!("../../../docs/docs-tool-embeds/uds-protocol.md"),
    ),
    (
        "models-providers",
        include_str!("../../../docs/docs-tool-embeds/runtime-models-providers.md"),
    ),
    (
        "contributor-cookbooks",
        include_str!("../../../docs/docs-tool-embeds/contributor-cookbooks.md"),
    ),
];

/// Normalize a requested doc name: strip a leading `docs/` or
/// `docs-tool-embeds/`, a trailing `.md`, lowercase, trim. So `subagents`,
/// `subagents.md`, and `docs/subagents.md` all resolve to the same doc.
fn normalize_name(raw: &str) -> String {
    raw.trim()
        .trim_start_matches("docs/docs-tool-embeds/")
        .trim_start_matches("docs-tool-embeds/")
        .trim_start_matches("docs/")
        .trim_end_matches(".md")
        .to_ascii_lowercase()
}

/// Look up an embedded doc by (normalized) name.
pub fn lookup_doc(name: &str) -> Option<&'static str> {
    let key = normalize_name(name);
    lookup_embedded_doc(&key)
}

fn lookup_embedded_doc(key: &str) -> Option<&'static str> {
    EMBEDDED_DOCS
        .iter()
        .find(|(n, _)| *n == key)
        .map(|(_, body)| *body)
}

/// A markdown bullet list of the available doc names, for listing / errors.
fn available_listing() -> String {
    let mut out = String::from("Available quecto capability docs:\n");
    for (name, _) in EMBEDDED_DOCS {
        out.push_str("- ");
        out.push_str(name);
        out.push('\n');
    }
    out.push_str("\nRead one with: docs {\"name\": \"subagents\"}");
    out
}

/// Tool that serves the embedded capability docs by name (CWD-independent).
#[derive(Debug, Default)]
pub struct DocsTool;

impl DocsTool {
    pub fn new() -> Self {
        Self
    }
}

impl Tool for DocsTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "docs".into(),
            description: "Read quecto's own capability docs, embedded in the binary and \
                available from any directory. Call with no name (or {}) to list \
                docs; pass a name to read one. Use this instead of reading \
                docs/*.md from disk. Example: docs {\"name\": \"subagents\"}"
                .into(),
            parameters_schema: r#"{"type":"object","properties":{"name":{"type":"string","description":"Doc to read, e.g. \"subagents\" or \"workflow\" (a docs/ prefix or .md suffix is accepted). Omit to list all available docs."}}}"#.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let name = serde_json::from_str::<serde_json::Value>(arguments)
            .ok()
            .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_string))
            .filter(|s| !s.trim().is_empty());

        Box::pin(async move {
            let Some(name) = name else {
                // No name → list available docs.
                return Ok(ToolResult {
                    content: available_listing(),
                    is_error: false,
                    image_blocks: vec![],
                });
            };
            let key = normalize_name(&name);
            if let Some(body) = lookup_embedded_doc(&key) {
                return Ok(ToolResult {
                    content: body.to_string(),
                    is_error: false,
                    image_blocks: vec![],
                });
            }
            Ok(ToolResult {
                content: format!("No embedded doc named '{name}'.\n\n{}", available_listing()),
                is_error: true,
                image_blocks: vec![],
            })
        })
    }
}

#[cfg(test)]
#[path = "docs_tests.rs"]
mod tests;
