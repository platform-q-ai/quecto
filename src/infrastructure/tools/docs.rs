// DocsTool: serves quecto's own capability docs, embedded in the binary.
//
// The capability docs (docs/*.md) are baked in at compile time via include_str!
// so they are reachable from ANY working directory — the prior guidance to
// `read docs/quecto.md` broke whenever quecto ran outside its own checkout
// (the path resolved relative to the agent's workspace/CWD).

use crate::domain::error::DomainError;
use crate::domain::skill::SkillLoader;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::persistence::skill_loader::FileSkillLoader;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

/// The embedded capability docs, keyed by short name (no `docs/` prefix, no
/// `.md` suffix). These are the agent-facing guides the docs-retrieval policy
/// points at; design PRDs are intentionally not embedded.
///
/// `include_str!` paths are relative to this source file
/// (`src/infrastructure/tools/`), so `../../../docs/` is the repo `docs/` dir.
/// A renamed/removed doc fails the build — the embed cannot silently drift.
const EMBEDDED_DOCS: &[(&str, &str)] = &[
    ("quecto", include_str!("../../../docs/quecto.md")),
    ("subagents", include_str!("../../../docs/subagents.md")),
    ("workflow", include_str!("../../../docs/workflow.md")),
    ("extensions", include_str!("../../../docs/extensions.md")),
    ("sessions", include_str!("../../../docs/sessions.md")),
    (
        "disable-tools",
        include_str!("../../../docs/disable-tools.md"),
    ),
    (
        "uds-protocol",
        include_str!("../../../docs/uds-protocol.md"),
    ),
    (
        "getting-started",
        include_str!("../../../docs/getting-started.md"),
    ),
    (
        "models-providers",
        include_str!("../../../docs/runtime-models-providers.md"),
    ),
    ("readme", include_str!("../../../README.md")),
];

/// Normalize a requested doc name: strip a leading `docs/` and a trailing
/// `.md`, lowercase, trim. So `quecto`, `quecto.md`, and `docs/quecto.md` all
/// resolve to the same doc.
fn normalize_name(raw: &str) -> String {
    raw.trim()
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

fn skill_doc_name(name: &str) -> String {
    format!("skills/{name}")
}

/// A markdown bullet list of the available doc names, for listing / errors.
fn available_listing(skill_docs: &[(String, String)]) -> String {
    let mut out = String::from("Available quecto capability docs:\n");
    for (name, _) in EMBEDDED_DOCS {
        out.push_str("- ");
        out.push_str(name);
        out.push('\n');
    }
    if !skill_docs.is_empty() {
        out.push_str("\nAvailable legacy skill knowledge docs:\n");
        for (name, description) in skill_docs {
            out.push_str("- ");
            out.push_str(name);
            if !description.is_empty() {
                out.push_str(" — ");
                out.push_str(description);
            }
            out.push('\n');
        }
    }
    out.push_str("\nRead one with: docs {\"name\": \"quecto\"}");
    out
}

/// Tool that serves the embedded capability docs by name (CWD-independent).
#[derive(Debug, Default)]
pub struct DocsTool {
    legacy_skills_workspace: Option<PathBuf>,
}

impl DocsTool {
    pub fn new() -> Self {
        Self {
            legacy_skills_workspace: None,
        }
    }

    pub fn with_workspace(workspace: impl AsRef<Path>) -> Self {
        Self {
            legacy_skills_workspace: Some(workspace.as_ref().to_path_buf()),
        }
    }

    fn legacy_skill_docs(&self) -> Vec<(String, String)> {
        let Some(workspace) = &self.legacy_skills_workspace else {
            return Vec::new();
        };
        FileSkillLoader::new(workspace)
            .list()
            .unwrap_or_default()
            .into_iter()
            .map(|skill| (skill_doc_name(&skill.name), skill.description))
            .collect()
    }

    fn lookup_legacy_skill(&self, key: &str) -> Option<String> {
        let skill_name = key.strip_prefix("skills/")?;
        let workspace = self.legacy_skills_workspace.as_ref()?;
        let skill = FileSkillLoader::new(workspace).load(skill_name).ok()??;
        if skill.content.is_empty() {
            None
        } else {
            Some(skill.content)
        }
    }
}

impl Tool for DocsTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "docs".into(),
            description: "Read quecto's own capability docs, embedded in the binary and \
                available from any directory, plus legacy workspace skills as on-demand \
                knowledge docs named skills/<name>. Call with no name (or {}) to list \
                docs; pass a name to read one. Start with \"quecto\". Use this instead \
                of reading docs/*.md from disk. Example: docs {\"name\": \"subagents\"}"
                .into(),
            parameters_schema: r#"{"type":"object","properties":{"name":{"type":"string","description":"Doc to read, e.g. \"quecto\", \"subagents\", or \"skills/review\" (a docs/ prefix or .md suffix is accepted). Omit to list all available docs."}}}"#.into(),
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
            let skill_docs = self.legacy_skill_docs();
            let Some(name) = name else {
                // No name → list available docs.
                return Ok(ToolResult {
                    content: available_listing(&skill_docs),
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
            if let Some(body) = self.lookup_legacy_skill(&key) {
                return Ok(ToolResult {
                    content: body,
                    is_error: false,
                    image_blocks: vec![],
                });
            }
            Ok(ToolResult {
                content: format!(
                    "No embedded doc or legacy skill knowledge doc named '{name}'.\n\n{}",
                    available_listing(&skill_docs)
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
