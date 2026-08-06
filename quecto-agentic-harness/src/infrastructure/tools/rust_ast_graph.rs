use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::rust_ast_graph_hits::{RefHit, UseHit, hit_for};
use super::rust_ast_graph_parse::build_graph;
use super::rust_ast_graph_text::{line_col, to_json, workspace_crates};
use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::infrastructure::security::sandbox::Sandbox;

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
    "depth":{"type":"integer","minimum":0,"maximum":5,"description":"Traversal depth for neighbors (default 1)"},
    "include_bodies":{"type":"boolean","description":"Include declaration bodies where available (default false)"},
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
            if let Err(e) = validate_action_args(&args) {
                return Ok(error(e));
            }
            let limit = args.limit.unwrap_or(50).clamp(1, 200);
            let depth = args.depth.unwrap_or(1).clamp(0, 5);
            let include_bodies = args.include_bodies.unwrap_or(false);
            let snippet_lines = if include_bodies {
                args.snippet_lines.unwrap_or(20).clamp(0, 20)
            } else {
                args.snippet_lines.unwrap_or(3).clamp(0, 20)
            };
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
                    graph.neighbors(sym, limit, depth)
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
                    graph.query(q, limit, snippet_lines, include_bodies)
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
    depth: Option<usize>,
    include_bodies: Option<bool>,
    snippet_lines: Option<usize>,
}

fn validate_action_args(args: &Args) -> Result<(), String> {
    match args.action.as_str() {
        "overview" => Ok(()),
        "find_symbol" if args.symbol.is_none() => {
            Err("missing required field 'symbol' for find_symbol".into())
        }
        "find_symbol" => Ok(()),
        "neighbors" if args.symbol.is_none() => {
            Err("missing required field 'symbol' for neighbors".into())
        }
        "neighbors" => Ok(()),
        "references" if args.symbol.is_none() => {
            Err("missing required field 'symbol' for references".into())
        }
        "references" => Ok(()),
        "calls" if args.symbol.is_none() => Err("missing required field 'symbol' for calls".into()),
        "calls" => Ok(()),
        "query" if args.query.is_none() => {
            Err("missing required field 'query' for query action".into())
        }
        "query" => Ok(()),
        other => Err(format!(
            "unsupported action '{other}'. Expected overview, find_symbol, neighbors, references, calls, or query"
        )),
    }
}

fn resolve_scope(
    workspace: &Path,
    sandbox: &Sandbox,
    raw: Option<&str>,
) -> Result<PathBuf, String> {
    let raw = raw.unwrap_or(".");
    let path = crate::infrastructure::tools::path_utils::resolve_to_cwd(raw, workspace);
    sandbox
        .validate_path(&path.to_string_lossy())
        .map_err(|e| e.to_string())
}

fn error(content: impl Into<String>) -> ToolResult {
    ToolResult {
        content: content.into(),
        is_error: true,
        image_blocks: vec![],
    }
}

#[derive(Default)]
pub(super) struct Graph {
    pub(super) workspace: PathBuf,
    pub(super) files: Vec<RustFile>,
    pub(super) symbols: Vec<Symbol>,
    pub(super) diagnostics: Vec<Diagnostic>,
}

#[derive(Clone)]
pub(super) struct RustFile {
    pub(super) rel: String,
    pub(super) text: String,
    pub(super) masked: String,
    pub(super) module: String,
}

#[derive(Clone, Serialize)]
pub(super) struct Symbol {
    pub(super) id: String,
    pub(super) name: String,
    pub(super) qualified_path: String,
    pub(super) kind: String,
    pub(super) visibility: String,
    pub(super) signature: String,
    pub(super) location: Location,
    pub(super) snippet: String,
    pub(super) module: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) trait_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) for_type: Option<String>,
}

#[derive(Clone, Serialize)]
pub(super) struct Location {
    pub(super) file: String,
    pub(super) line: usize,
    pub(super) column: usize,
    pub(super) byte_start: usize,
    pub(super) byte_end: usize,
}
#[derive(Serialize)]
pub(super) struct Diagnostic {
    pub(super) file: String,
    pub(super) message: String,
}

pub(super) fn symbol_matches(symbol: &Symbol, needle: &str) -> bool {
    symbol.id == needle
        || symbol.name == needle
        || qualified_path_segment_matches(&symbol.qualified_path, needle)
}

pub(super) fn qualified_path_segment_matches(qualified_path: &str, needle: &str) -> bool {
    qualified_path == needle || qualified_path.ends_with(&format!("::{needle}"))
}

pub(super) struct SymbolParts<'a> {
    pub(super) file: &'a RustFile,
    pub(super) name: &'a str,
    pub(super) kind: &'a str,
    pub(super) visibility: &'a str,
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) trait_name: Option<String>,
    pub(super) for_type: Option<String>,
    pub(super) snippet_lines: usize,
}

fn symbol_for_response(symbol: &Symbol, snippet_lines: usize, include_bodies: bool) -> Symbol {
    let mut out = symbol.clone();
    if !include_bodies || snippet_lines == 0 {
        out.snippet.clear();
    }
    out
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
            .filter(|s| symbol_matches(s, needle))
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
                .filter(|s| qualified_path_segment_matches(&s.qualified_path, needle))
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
    fn neighbors(&self, needle: &str, limit: usize, depth: usize) -> Result<String, String> {
        #[derive(Serialize)]
        struct Neighbors<'a> {
            derived_from: &'static str,
            target: &'a Symbol,
            containing_module: &'a str,
            depth: usize,
            child_declarations: Vec<&'a Symbol>,
            imports_uses: Vec<UseHit>,
            implementations: Vec<&'a Symbol>,
            trait_relationships: Vec<&'a Symbol>,
            syntactic_call_sites: Vec<RefHit>,
            diagnostics: &'a [Diagnostic],
        }
        let target = self.select_symbol(needle)?;
        let child_declarations = if depth == 0 {
            Vec::new()
        } else {
            self.symbols
                .iter()
                .filter(|s| {
                    s.module == target.name || s.module.ends_with(&format!("::{}", target.name))
                })
                .take(limit)
                .collect()
        };
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
        let syntactic_call_sites = if depth == 0 {
            Vec::new()
        } else {
            self.reference_hits(&target.name, false, limit.min(25), 2, true)
        };
        Ok(to_json(&Neighbors {
            derived_from: "Rust syntax/text (not compiler name/type resolution)",
            target,
            containing_module: &target.module,
            depth,
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
    fn query(
        &self,
        q: &str,
        limit: usize,
        snippet_lines: usize,
        include_bodies: bool,
    ) -> Result<String, String> {
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
                    results.push(
                        serde_json::to_value(symbol_for_response(s, snippet_lines, include_bodies))
                            .unwrap(),
                    );
                }
            }
            "trait_impls" => {
                for s in self
                    .symbols
                    .iter()
                    .filter(|s| s.kind == "impl" && s.trait_name.is_some())
                    .take(limit)
                {
                    results.push(
                        serde_json::to_value(symbol_for_response(s, snippet_lines, include_bodies))
                            .unwrap(),
                    );
                }
            }
            "public_api" => {
                for s in self
                    .symbols
                    .iter()
                    .filter(|s| s.visibility.starts_with("pub"))
                    .take(limit)
                {
                    results.push(
                        serde_json::to_value(symbol_for_response(s, snippet_lines, include_bodies))
                            .unwrap(),
                    );
                }
            }
            "functions" => {
                for s in self.symbols.iter().filter(|s| s.kind == "fn").take(limit) {
                    results.push(
                        serde_json::to_value(symbol_for_response(s, snippet_lines, include_bodies))
                            .unwrap(),
                    );
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
