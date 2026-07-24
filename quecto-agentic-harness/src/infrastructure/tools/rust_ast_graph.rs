use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::path_utils::resolve_to_cwd;

use super::rust_ast_graph_hits::{RefHit, UseHit, hit_for};
use super::rust_ast_graph_text::{
    line_col, line_end, mask_comments_and_strings, module_path, rel_path, snippet, to_json,
    workspace_crates,
};

#[derive(Debug)]
pub struct RustAstGraphTool {
    workspace: Arc<PathBuf>,
    sandbox: Arc<Sandbox>,
}

impl RustAstGraphTool {
    pub fn new(workspace: Arc<PathBuf>, sandbox: Arc<Sandbox>) -> Self {
        Self { workspace, sandbox }
    }
}

impl Tool for RustAstGraphTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "rust_ast_graph".into(),
            description: "Rust-aware syntax graph navigation for the current workspace. Actions: overview, find_symbol, neighbors, references, calls, query. Returns syntax-derived (not compiler/type-proven) locations, declarations, imports, impls, call sites, and compact snippets. Example: {\"action\":\"find_symbol\",\"symbol\":\"ToolRegistryImpl\",\"limit\":10}".into(),
            parameters_schema: r#"{
  "type":"object",
  "properties":{
    "action":{"type":"string","enum":["overview","find_symbol","neighbors","references","calls","query"],"description":"Operation to run"},
    "path":{"type":"string","description":"Optional workspace-relative file or directory scope"},
    "symbol":{"type":"string","description":"Symbol name, qualified path, or stable node id"},
    "query":{"type":"string","enum":["async_functions","unsafe_blocks","trait_impls","public_api","functions"],"description":"Structural query for action=query"},
    "raw_text":{"type":"boolean","description":"Include comments and string literals for reference-like matching (default false)"},
    "limit":{"type":"integer","minimum":1,"maximum":200,"description":"Maximum result count (default 50)"},
    "snippet_lines":{"type":"integer","minimum":0,"maximum":20,"description":"Context lines per result (default 3)"}
  },
  "required":["action"]
}"#.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let arguments = arguments.to_string();
        Box::pin(async move {
            let args: Args = match serde_json::from_str(&arguments) {
                Ok(v) => v,
                Err(e) => {
                    return Ok(error(format!(
                        "invalid JSON arguments: {e}. Example: {{\"action\":\"overview\"}}"
                    )));
                }
            };
            let limit = args.limit.unwrap_or(50).clamp(1, 200);
            let snippet_lines = args.snippet_lines.unwrap_or(3).clamp(0, 20);
            let scope = match resolve_scope(&self.workspace, &self.sandbox, args.path.as_deref()) {
                Ok(p) => p,
                Err(e) => return Ok(error(e)),
            };
            let graph = match build_graph(&scope, &self.workspace, &self.sandbox, snippet_lines) {
                Ok(g) => g,
                Err(e) => return Ok(error(e)),
            };
            let response = match args.action.as_str() {
                "overview" => Ok(graph.overview(limit)),
                "find_symbol" => {
                    let Some(sym) = args.symbol.as_deref() else {
                        return Ok(error("missing required field 'symbol' for find_symbol"));
                    };
                    Ok(graph.find_symbol(sym, limit))
                }
                "neighbors" => {
                    let Some(sym) = args.symbol.as_deref() else {
                        return Ok(error("missing required field 'symbol' for neighbors"));
                    };
                    graph.neighbors(sym, limit)
                }
                "references" => {
                    let Some(sym) = args.symbol.as_deref() else {
                        return Ok(error("missing required field 'symbol' for references"));
                    };
                    Ok(graph.references(
                        sym,
                        args.raw_text.unwrap_or(false),
                        limit,
                        snippet_lines,
                        false,
                    ))
                }
                "calls" => {
                    let Some(sym) = args.symbol.as_deref() else {
                        return Ok(error("missing required field 'symbol' for calls"));
                    };
                    Ok(graph.references(
                        sym,
                        args.raw_text.unwrap_or(false),
                        limit,
                        snippet_lines,
                        true,
                    ))
                }
                "query" => {
                    let Some(q) = args.query.as_deref() else {
                        return Ok(error("missing required field 'query' for query action"));
                    };
                    graph.query(q, limit, snippet_lines)
                }
                other => Err(format!(
                    "unsupported action '{other}'. Expected overview, find_symbol, neighbors, references, calls, or query"
                )),
            };
            match response {
                Ok(content) => Ok(ToolResult {
                    content,
                    is_error: false,
                    image_blocks: vec![],
                }),
                Err(e) => Ok(error(e)),
            }
        })
    }
}

#[derive(Deserialize)]
struct Args {
    action: String,
    path: Option<String>,
    symbol: Option<String>,
    query: Option<String>,
    raw_text: Option<bool>,
    limit: Option<usize>,
    snippet_lines: Option<usize>,
}

fn error(content: impl Into<String>) -> ToolResult {
    ToolResult {
        content: content.into(),
        is_error: true,
        image_blocks: vec![],
    }
}

fn resolve_scope(
    workspace: &Path,
    sandbox: &Sandbox,
    raw: Option<&str>,
) -> Result<PathBuf, String> {
    let raw = raw.unwrap_or(".");
    let path = resolve_to_cwd(raw, workspace);
    sandbox
        .validate_path(&path.to_string_lossy())
        .map_err(|e| e.to_string())
}

#[derive(Default)]
struct Graph {
    workspace: PathBuf,
    files: Vec<RustFile>,
    symbols: Vec<Symbol>,
    diagnostics: Vec<Diagnostic>,
}

#[derive(Clone)]
pub(super) struct RustFile {
    pub(super) rel: String,
    pub(super) text: String,
    masked: String,
    module: String,
}

#[derive(Clone, Serialize)]
struct Symbol {
    id: String,
    name: String,
    qualified_path: String,
    kind: String,
    visibility: String,
    signature: String,
    location: Location,
    snippet: String,
    module: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    trait_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    for_type: Option<String>,
}

#[derive(Clone, Serialize)]
struct Location {
    file: String,
    line: usize,
    column: usize,
    byte_start: usize,
    byte_end: usize,
}
#[derive(Serialize)]
struct Diagnostic {
    file: String,
    message: String,
}

const MAX_RUST_FILES: usize = 2_000;
const MAX_TOTAL_BYTES: u64 = 25 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

fn build_graph(
    scope: &Path,
    workspace: &Path,
    sandbox: &Sandbox,
    snippet_lines: usize,
) -> Result<Graph, String> {
    let mut graph = Graph {
        workspace: workspace.to_path_buf(),
        ..Graph::default()
    };
    let mut files = Vec::new();
    collect_rs(scope, sandbox, &mut files, &mut graph.diagnostics)?;
    files.sort();
    let mut total_bytes = 0u64;
    for abs in files {
        let rel = rel_path(&abs, workspace);
        let Ok(validated) = sandbox.validate_path(&abs.to_string_lossy()) else {
            graph.diagnostics.push(Diagnostic {
                file: rel,
                message: "skipped path outside sandbox".into(),
            });
            continue;
        };
        let size = std::fs::metadata(&validated).map(|m| m.len()).unwrap_or(0);
        if size > MAX_FILE_BYTES || total_bytes.saturating_add(size) > MAX_TOTAL_BYTES {
            graph.diagnostics.push(Diagnostic {
                file: rel,
                message: "skipped by rust_ast_graph size limit".into(),
            });
            continue;
        }
        total_bytes += size;
        match std::fs::read_to_string(&validated) {
            Ok(text) => {
                let masked = mask_comments_and_strings(&text);
                let module = module_path(&abs, workspace);
                let rf = RustFile {
                    rel: rel.clone(),
                    text,
                    masked,
                    module,
                };
                let mut symbols = parse_symbols(&rf, snippet_lines);
                graph.symbols.append(&mut symbols);
                graph.files.push(rf);
            }
            Err(e) => graph.diagnostics.push(Diagnostic {
                file: rel,
                message: format!("failed to read file: {e}"),
            }),
        }
    }
    Ok(graph)
}

fn collect_rs(
    path: &Path,
    sandbox: &Sandbox,
    out: &mut Vec<PathBuf>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), String> {
    sandbox
        .validate_path(&path.to_string_lossy())
        .map_err(|e| e.to_string())?;
    if path.is_file() {
        if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            push_candidate(path, sandbox, out, diagnostics);
        }
        return Ok(());
    }
    let entries =
        std::fs::read_dir(path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    for entry in entries.flatten() {
        let p = entry.path();
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name == ".git" || name == "target" || name == ".quecto" {
            continue;
        }
        if p.is_dir() {
            collect_rs(&p, sandbox, out, diagnostics)?;
        } else if p.extension().and_then(|e| e.to_str()) == Some("rs") {
            push_candidate(&p, sandbox, out, diagnostics);
        }
        if out.len() >= MAX_RUST_FILES {
            diagnostics.push(Diagnostic {
                file: rel_path(path, path),
                message: format!("stopped after MAX_RUST_FILES={MAX_RUST_FILES}"),
            });
            break;
        }
    }
    Ok(())
}

fn push_candidate(
    path: &Path,
    sandbox: &Sandbox,
    out: &mut Vec<PathBuf>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    match sandbox.validate_path(&path.to_string_lossy()) {
        Ok(validated) => out.push(validated),
        Err(e) => diagnostics.push(Diagnostic {
            file: path.to_string_lossy().into_owned(),
            message: format!("skipped path outside sandbox: {e}"),
        }),
    }
}

fn parse_symbols(file: &RustFile, snippet_lines: usize) -> Vec<Symbol> {
    let item_re = Regex::new(r"(?m)^\s*((?:pub(?:\([^)]*\))?\s+)?)((?:async\s+)?(?:unsafe\s+)?)\b(fn|struct|enum|trait|type|const|static|mod)\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
    let impl_re =
        Regex::new(r"(?m)^\s*(unsafe\s+)?impl(?:\s*<[^>{;]*>)?\s+([^\n{;]+?)\s*\{").unwrap();
    let mut syms = Vec::new();
    for cap in item_re.captures_iter(&file.masked) {
        let m = cap.get(0).unwrap();
        let name = cap[4].to_string();
        let kind = cap[3].to_string();
        let visibility = if cap
            .get(1)
            .map(|m| !m.as_str().trim().is_empty())
            .unwrap_or(false)
        {
            cap[1].trim().to_string()
        } else {
            "private".into()
        };
        syms.push(make_symbol(SymbolParts {
            file,
            name: &name,
            kind: &kind,
            visibility: &visibility,
            start: m.start(),
            end: m.end(),
            trait_name: None,
            for_type: None,
            snippet_lines,
        }));
    }
    for cap in impl_re.captures_iter(&file.masked) {
        let m = cap.get(0).unwrap();
        let target = cap[2].trim();
        let (trait_name, for_type, name) = if let Some((tr, ty)) = split_impl_target(target) {
            (
                Some(tr.to_string()),
                Some(ty.to_string()),
                format!("impl {tr} for {ty}"),
            )
        } else {
            (None, Some(target.to_string()), format!("impl {target}"))
        };
        syms.push(make_symbol(SymbolParts {
            file,
            name: &name,
            kind: "impl",
            visibility: "inherent/syntactic",
            start: m.start(),
            end: m.end(),
            trait_name,
            for_type,
            snippet_lines,
        }));
    }
    syms
}

fn split_impl_target(target: &str) -> Option<(&str, &str)> {
    let parts: Vec<&str> = target.split(" for ").collect();
    if parts.len() == 2 {
        Some((parts[0].trim(), parts[1].trim()))
    } else {
        None
    }
}

struct SymbolParts<'a> {
    file: &'a RustFile,
    name: &'a str,
    kind: &'a str,
    visibility: &'a str,
    start: usize,
    end: usize,
    trait_name: Option<String>,
    for_type: Option<String>,
    snippet_lines: usize,
}

fn make_symbol(parts: SymbolParts<'_>) -> Symbol {
    let file = parts.file;
    let (line, col) = line_col(&file.text, parts.start);
    let qp = if file.module.is_empty() || file.module == "crate" {
        parts.name.to_string()
    } else {
        format!("{}::{}", file.module, parts.name)
    };
    let id = format!(
        "{}:{}:{}:{}:{}",
        file.rel, parts.start, parts.end, parts.kind, parts.name
    );
    let signature = file.text[parts.start..line_end(&file.text, parts.start)]
        .trim()
        .to_string();
    Symbol {
        id,
        name: parts.name.to_string(),
        qualified_path: qp,
        kind: parts.kind.to_string(),
        visibility: parts.visibility.to_string(),
        signature,
        location: Location {
            file: file.rel.clone(),
            line,
            column: col,
            byte_start: parts.start,
            byte_end: parts.end,
        },
        snippet: snippet(&file.text, line, parts.snippet_lines),
        module: file.module.clone(),
        trait_name: parts.trait_name,
        for_type: parts.for_type,
    }
}

impl Graph {
    fn overview(&self, limit: usize) -> String {
        #[derive(Serialize)]
        struct Overview<'a> {
            derived_from: &'static str,
            crates: Vec<String>,
            files: usize,
            modules: Vec<&'a str>,
            top_level_declarations: Vec<&'a Symbol>,
            diagnostics: &'a [Diagnostic],
        }
        let crates = workspace_crates(&self.workspace);
        let mut modules: Vec<&str> = self.files.iter().map(|f| f.module.as_str()).collect();
        modules.sort_unstable();
        modules.dedup();
        let decls: Vec<&Symbol> = self
            .symbols
            .iter()
            .filter(|s| s.kind != "impl")
            .take(limit)
            .collect();
        to_json(&Overview {
            derived_from: "Rust syntax/text (not compiler name/type resolution)",
            crates,
            files: self.files.len(),
            modules,
            top_level_declarations: decls,
            diagnostics: &self.diagnostics,
        })
    }
    fn find_symbol(&self, needle: &str, limit: usize) -> String {
        #[derive(Serialize)]
        struct Resp<'a> {
            derived_from: &'static str,
            query: &'a str,
            ambiguous: bool,
            matches: Vec<&'a Symbol>,
            diagnostics: &'a [Diagnostic],
        }
        let mut matches: Vec<&Symbol> = self
            .symbols
            .iter()
            .filter(|s| s.id == needle || s.name == needle || s.qualified_path.ends_with(needle))
            .collect();
        matches.sort_by_key(|s| (&s.qualified_path, &s.location.file, s.location.line));
        let ambiguous = matches.len() > 1;
        matches.truncate(limit);
        to_json(&Resp {
            derived_from: "Rust syntax/text (not compiler name/type resolution)",
            query: needle,
            ambiguous,
            matches,
            diagnostics: &self.diagnostics,
        })
    }
    fn select_symbol(&self, needle: &str) -> Result<&Symbol, String> {
        let exact: Vec<&Symbol> = self
            .symbols
            .iter()
            .filter(|s| s.id == needle || s.name == needle || s.qualified_path == needle)
            .collect();
        let matches = if exact.is_empty() {
            self.symbols
                .iter()
                .filter(|s| s.qualified_path.ends_with(needle))
                .collect()
        } else {
            exact
        };
        match matches.len() {
            0 => Err(format!("no Rust symbol found for '{needle}'")),
            1 => Ok(matches[0]),
            n => Err(format!(
                "ambiguous symbol '{needle}' matched {n} declarations; run find_symbol and retry with a stable id"
            )),
        }
    }
    fn neighbors(&self, needle: &str, limit: usize) -> Result<String, String> {
        #[derive(Serialize)]
        struct Neighbors<'a> {
            derived_from: &'static str,
            target: &'a Symbol,
            containing_module: &'a str,
            child_declarations: Vec<&'a Symbol>,
            imports_uses: Vec<UseHit>,
            implementations: Vec<&'a Symbol>,
            trait_relationships: Vec<&'a Symbol>,
            syntactic_call_sites: Vec<RefHit>,
            diagnostics: &'a [Diagnostic],
        }
        let target = self.select_symbol(needle)?;
        let child_declarations = self
            .symbols
            .iter()
            .filter(|s| {
                s.module == target.name || s.module.ends_with(&format!("::{}", target.name))
            })
            .take(limit)
            .collect();
        let imports_uses = self.uses_in_module(&target.module, limit);
        let implementations: Vec<&Symbol> = self
            .symbols
            .iter()
            .filter(|s| {
                s.kind == "impl"
                    && s.for_type
                        .as_deref()
                        .map(|t| t.contains(&target.name))
                        .unwrap_or(false)
            })
            .take(limit)
            .collect();
        let trait_relationships = implementations.clone();
        let syntactic_call_sites = self.reference_hits(&target.name, false, limit.min(25), 2, true);
        Ok(to_json(&Neighbors {
            derived_from: "Rust syntax/text (not compiler name/type resolution)",
            target,
            containing_module: &target.module,
            child_declarations,
            imports_uses,
            implementations,
            trait_relationships,
            syntactic_call_sites,
            diagnostics: &self.diagnostics,
        }))
    }
    fn references(
        &self,
        needle: &str,
        raw_text: bool,
        limit: usize,
        snippet_lines: usize,
        calls_only: bool,
    ) -> String {
        #[derive(Serialize)]
        struct Resp<'a> {
            derived_from: &'static str,
            target: String,
            raw_text: bool,
            calls_only: bool,
            results: Vec<RefHit>,
            diagnostics: &'a [Diagnostic],
        }
        let name = self
            .select_symbol(needle)
            .map(|s| s.name.clone())
            .unwrap_or_else(|_| needle.rsplit("::").next().unwrap_or(needle).to_string());
        let results = self.reference_hits(&name, raw_text, limit, snippet_lines, calls_only);
        to_json(&Resp {
            derived_from: "Rust syntax/text search; references/calls are lexical candidates, not compiler-resolved facts",
            target: name,
            raw_text,
            calls_only,
            results,
            diagnostics: &self.diagnostics,
        })
    }
    fn query(&self, q: &str, limit: usize, snippet_lines: usize) -> Result<String, String> {
        #[derive(Serialize)]
        struct Resp<'a> {
            derived_from: &'static str,
            query: &'a str,
            results: Vec<serde_json::Value>,
            diagnostics: &'a [Diagnostic],
        }
        let mut results = Vec::new();
        match q {
            "async_functions" => {
                for s in self
                    .symbols
                    .iter()
                    .filter(|s| s.kind == "fn" && s.signature.contains("async fn"))
                    .take(limit)
                {
                    results.push(serde_json::to_value(s).unwrap());
                }
            }
            "trait_impls" => {
                for s in self
                    .symbols
                    .iter()
                    .filter(|s| s.kind == "impl" && s.trait_name.is_some())
                    .take(limit)
                {
                    results.push(serde_json::to_value(s).unwrap());
                }
            }
            "public_api" => {
                for s in self
                    .symbols
                    .iter()
                    .filter(|s| s.visibility.starts_with("pub"))
                    .take(limit)
                {
                    results.push(serde_json::to_value(s).unwrap());
                }
            }
            "functions" => {
                for s in self.symbols.iter().filter(|s| s.kind == "fn").take(limit) {
                    results.push(serde_json::to_value(s).unwrap());
                }
            }
            "unsafe_blocks" => return Ok(self.unsafe_blocks(limit, snippet_lines)),
            _ => return Err(format!("unsupported query '{q}'")),
        }
        Ok(to_json(&Resp {
            derived_from: "Rust syntax/text (not compiler name/type resolution)",
            query: q,
            results,
            diagnostics: &self.diagnostics,
        }))
    }
    fn unsafe_blocks(&self, limit: usize, snippet_lines: usize) -> String {
        #[derive(Serialize)]
        struct Resp<'a> {
            derived_from: &'static str,
            query: &'static str,
            results: Vec<RefHit>,
            diagnostics: &'a [Diagnostic],
        }
        let re = Regex::new(r"\bunsafe\s*\{").unwrap();
        let mut hits = Vec::new();
        for f in &self.files {
            for m in re.find_iter(&f.masked) {
                if hits.len() >= limit {
                    break;
                }
                hits.push(hit_for(
                    f,
                    m.start(),
                    m.end(),
                    snippet_lines,
                    "unsafe block",
                ));
            }
        }
        to_json(&Resp {
            derived_from: "Rust syntax/text (not compiler name/type resolution)",
            query: "unsafe_blocks",
            results: hits,
            diagnostics: &self.diagnostics,
        })
    }
    fn uses_in_module(&self, module: &str, limit: usize) -> Vec<UseHit> {
        let re = Regex::new(r"(?m)^\s*use\s+([^;]+);").unwrap();
        let mut out = Vec::new();
        for f in self.files.iter().filter(|f| f.module == module) {
            for cap in re.captures_iter(&f.masked) {
                if out.len() >= limit {
                    return out;
                }
                let m = cap.get(0).unwrap();
                let (line, column) = line_col(&f.text, m.start());
                out.push(UseHit {
                    file: f.rel.clone(),
                    line,
                    column,
                    use_tree: cap[1].trim().to_string(),
                });
            }
        }
        out
    }
    fn reference_hits(
        &self,
        name: &str,
        raw_text: bool,
        limit: usize,
        snippet_lines: usize,
        calls_only: bool,
    ) -> Vec<RefHit> {
        let pat = if calls_only {
            format!(r"\b{}\s*\(", regex::escape(name))
        } else {
            format!(r"\b{}\b", regex::escape(name))
        };
        let re = Regex::new(&pat).unwrap();
        let mut hits = Vec::new();
        for f in &self.files {
            let hay = if raw_text { &f.text } else { &f.masked };
            for m in re.find_iter(hay) {
                if hits.len() >= limit {
                    return hits;
                }
                hits.push(hit_for(
                    f,
                    m.start(),
                    m.end(),
                    snippet_lines,
                    if calls_only {
                        "syntactic call candidate"
                    } else {
                        "lexical reference candidate"
                    },
                ));
            }
        }
        hits
    }
}

#[cfg(test)]
#[path = "rust_ast_graph_tests.rs"]
mod tests;
