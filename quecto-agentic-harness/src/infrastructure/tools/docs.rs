// DocsTool: Quecto's operating manual, embedded in the binary.
//
// Agent-facing guides live under `docs/docs-tool-embeds/` and are baked in at
// compile time via `include_str!` so they are reachable from ANY working
// directory — reading docs from disk breaks whenever quecto runs outside its
// own checkout (paths resolve relative to the agent's CWD).

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use std::future::Future;
use std::pin::Pin;

/// The embedded operating-manual pages, keyed by short name (no path prefix,
/// no `.md` suffix). Human-only docs (UDS protocol, sessions, cookbooks,
/// PRDs, ADRs, full reference manuals under `docs/`) are intentionally not
/// embedded.
///
/// `include_str!` paths are relative to this source file
/// (`src/infrastructure/tools/`), so `../../../docs/docs-tool-embeds/` is the
/// package embed folder. A renamed/removed doc fails the build — the embed
/// cannot silently drift.
const EMBEDDED_DOCS: &[(&str, &str)] = &[
    (
        "admission-broker",
        include_str!("../../../docs/docs-tool-embeds/admission-broker.md"),
    ),
    (
        "swarm",
        include_str!("../../../docs/docs-tool-embeds/swarm.md"),
    ),
    (
        "subagents",
        include_str!("../../../docs/docs-tool-embeds/subagents.md"),
    ),
    (
        "workflow",
        include_str!("../../../docs/docs-tool-embeds/workflow.md"),
    ),
    (
        "context",
        include_str!("../../../docs/docs-tool-embeds/context.md"),
    ),
    (
        "extending",
        include_str!("../../../docs/docs-tool-embeds/extending.md"),
    ),
    (
        "extensions",
        include_str!("../../../docs/docs-tool-embeds/extensions.md"),
    ),
    (
        "models",
        include_str!("../../../docs/docs-tool-embeds/models.md"),
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

/// First markdown H1 title in `body`, or `None` if missing.
fn doc_title(body: &str) -> Option<&str> {
    for line in body.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            let title = rest.trim();
            if !title.is_empty() {
                return Some(title);
            }
        }
    }
    None
}

/// Look up a shared embedded doc by normalized name.
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

/// Runtime role retained for constructor compatibility. All roles share the manual.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocsContentPolicy {
    Parent,
    Child,
}

/// Shared table of contents, independent of runtime role.
fn available_listing() -> String {
    let mut out = String::from(
        "Quecto operating manual (`docs` tool).\n\
         Call with no name to list pages; pass a name to read one.\n\
         Open manual pages only when needed.\n\n\
         Table of contents:\n",
    );
    for (name, body) in EMBEDDED_DOCS {
        let title = doc_title(body).unwrap_or(name);
        out.push_str("- ");
        out.push_str(name);
        out.push_str(" — ");
        out.push_str(title);
        out.push('\n');
    }
    out.push_str("\nRead one with: docs {\"name\": \"workflow\"}");
    out
}

/// Shared embedded operating manual (CWD-independent).
/// Tool availability remains owned by runtime profile policy.
#[derive(Debug)]
pub struct DocsTool;

impl Default for DocsTool {
    fn default() -> Self {
        Self::new()
    }
}

impl DocsTool {
    /// Top-level (parent) docs tool: shared operating manual.
    pub fn new() -> Self {
        Self::with_content_policy(DocsContentPolicy::Parent)
    }

    /// Child constructor; serves the same role-neutral manual.
    pub fn for_child_content() -> Self {
        Self::with_content_policy(DocsContentPolicy::Child)
    }

    /// Compatibility constructor; runtime role no longer filters manual content.
    pub fn with_content_policy(_content_policy: DocsContentPolicy) -> Self {
        Self
    }
}

impl Tool for DocsTool {
    fn definition(&self) -> ToolDefinition {
        let description = "Quecto operating manual (embedded in the binary, CWD-independent). \
            Call with no name (or {}) for the table of contents (name + title per page). \
            Pass a name to read one page. \
            Open deep-dive pages only when needed. Do not read docs from the filesystem. \
            Example: docs {\"name\": \"workflow\"}"
            .into();
        ToolDefinition {
            name: "docs".into(),
            description,
            parameters_schema: r#"{"type":"object","properties":{"name":{"type":"string","description":"Manual page to read, e.g. \"subagents\" or \"workflow\" (a docs/ prefix or .md suffix is accepted). Omit to list the table of contents."}}}"#.into(),
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
                return Ok(ToolResult {
                    content: available_listing(),
                    is_error: false,
                    image_blocks: vec![],
                    delivery_metadata: None,
                });
            };
            let key = normalize_name(&name);
            if let Some(body) = lookup_embedded_doc(&key) {
                return Ok(ToolResult {
                    content: body.to_string(),
                    is_error: false,
                    image_blocks: vec![],
                    delivery_metadata: None,
                });
            }
            Ok(ToolResult {
                content: format!("No embedded doc named '{name}'.\n\n{}", available_listing()),
                is_error: true,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

#[cfg(test)]
#[path = "docs_tests.rs"]
mod tests;
