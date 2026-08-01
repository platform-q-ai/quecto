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

/// Parent-coordination page. Visible only to top-level parent agents; omitted
/// for processes launched with the internal `--spawned` flag (#1319).
const PARENT_ONLY_DOC: &str = "quick-start";

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
        "quick-start",
        include_str!("../../../docs/docs-tool-embeds/quick-start.md"),
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

/// Whether `key` is a parent-coordination page filtered by child content policy.
fn is_parent_only_doc(key: &str) -> bool {
    key == PARENT_ONLY_DOC
}

/// Look up an embedded doc by (normalized) name. Does not apply spawned
/// visibility filtering — use [`DocsTool`] for agent-facing access.
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

/// Explicit content policy for embedded manual pages. This is deliberately
/// separate from tool availability/profile policy: every runtime may receive
/// the same `docs` tool according to profile scope, while child content policy
/// keeps parent-coordination quick-start material out of spawned child context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocsContentPolicy {
    Parent,
    Child,
}

impl DocsContentPolicy {
    const fn is_child(self) -> bool {
        matches!(self, Self::Child)
    }
}

/// Table of contents for the given content policy.
fn available_listing(policy: DocsContentPolicy) -> String {
    let child = policy.is_child();
    let intro = if child {
        "Quecto operating manual (`docs` tool).\n\
         Call with no name to list pages; pass a name to read one.\n\
         Open manual pages only when that knowledge is needed. Keep context lean.\n\n\
         Table of contents:\n"
    } else {
        "Quecto operating manual (`docs` tool).\n\
         Call with no name to list pages; pass a name to read one.\n\
         Start with `quick-start` for parent coordination; open other pages only when needed.\n\n\
         Table of contents:\n"
    };
    let mut out = String::from(intro);
    for (name, body) in EMBEDDED_DOCS {
        if child && is_parent_only_doc(name) {
            continue;
        }
        let title = doc_title(body).unwrap_or(name);
        out.push_str("- ");
        out.push_str(name);
        out.push_str(" — ");
        out.push_str(title);
        out.push('\n');
    }
    if child {
        out.push_str("\nRead one with: docs {\"name\": \"workflow\"}");
    } else {
        out.push_str("\nRead one with: docs {\"name\": \"quick-start\"}");
    }
    out
}

/// Tool that serves the embedded operating manual by name (CWD-independent).
///
/// Child runtimes may use child content policy, which omits the parent-only
/// `quick-start` page from the TOC and rejects direct lookup — including aliases
/// like `docs/quick-start.md` (#1319). Tool availability itself is not decided
/// here; it is owned by runtime profile policy.
#[derive(Debug)]
pub struct DocsTool {
    content_policy: DocsContentPolicy,
}

impl Default for DocsTool {
    fn default() -> Self {
        Self::new()
    }
}

impl DocsTool {
    /// Top-level (parent) docs tool: full manual including `quick-start`.
    pub fn new() -> Self {
        Self::with_content_policy(DocsContentPolicy::Parent)
    }

    /// Docs tool with child content filtering. Availability remains profile-driven.
    pub fn for_child_content() -> Self {
        Self::with_content_policy(DocsContentPolicy::Child)
    }

    /// Construct with an explicit content policy (#1319/#1334 Phase 3).
    pub fn with_content_policy(content_policy: DocsContentPolicy) -> Self {
        Self { content_policy }
    }
}

impl Tool for DocsTool {
    fn definition(&self) -> ToolDefinition {
        let description = if self.content_policy.is_child() {
            "Quecto operating manual (embedded in the binary, CWD-independent). \
                Call with no name (or {}) for the table of contents (name + title per page). \
                Pass a name to read one page. Open deep-dive pages only when needed. \
                Do not read docs from the filesystem. Example: docs {\"name\": \"workflow\"}"
                .into()
        } else {
            "Quecto operating manual (embedded in the binary, CWD-independent). \
                Call with no name (or {}) for the table of contents (name + title per page). \
                Pass a name to read one page. Start with \"quick-start\" for parent-agent \
                coordination; open deep-dive pages only when needed. Do not read docs from \
                the filesystem. Example: docs {\"name\": \"quick-start\"}"
                .into()
        };
        ToolDefinition {
            name: "docs".into(),
            description,
            parameters_schema: r#"{"type":"object","properties":{"name":{"type":"string","description":"Manual page to read, e.g. \"quick-start\" or \"workflow\" (a docs/ prefix or .md suffix is accepted). Omit to list the table of contents."}}}"#.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let content_policy = self.content_policy;
        let name = serde_json::from_str::<serde_json::Value>(arguments)
            .ok()
            .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_string))
            .filter(|s| !s.trim().is_empty());

        Box::pin(async move {
            let Some(name) = name else {
                return Ok(ToolResult {
                    content: available_listing(content_policy),
                    is_error: false,
                    image_blocks: vec![],
                });
            };
            let key = normalize_name(&name);
            if content_policy.is_child() && is_parent_only_doc(&key) {
                return Ok(ToolResult {
                    content: format!(
                        "No embedded doc named '{name}'.\n\n{}",
                        available_listing(DocsContentPolicy::Child)
                    ),
                    is_error: true,
                    image_blocks: vec![],
                });
            }
            if let Some(body) = lookup_embedded_doc(&key) {
                return Ok(ToolResult {
                    content: body.to_string(),
                    is_error: false,
                    image_blocks: vec![],
                });
            }
            Ok(ToolResult {
                content: format!(
                    "No embedded doc named '{name}'.\n\n{}",
                    available_listing(content_policy)
                ),
                is_error: true,
                image_blocks: vec![],
            })
        })
    }
}

#[cfg(test)]
#[path = "docs_tests.rs"]
mod tests;
