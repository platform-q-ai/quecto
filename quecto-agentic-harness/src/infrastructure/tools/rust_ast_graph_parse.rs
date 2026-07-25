use std::collections::HashSet;
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::infrastructure::security::sandbox::Sandbox;

use super::rust_ast_graph::{Diagnostic, Graph, RustFile, Symbol, SymbolParts};
use super::rust_ast_graph_text::{
    line_col, line_end, mask_comments_and_strings, module_path, rel_path, snippet,
};

const MAX_RUST_FILES: usize = 2_000;
const MAX_TOTAL_BYTES: u64 = 25 * 1024 * 1024;
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

pub(super) fn build_graph(
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
    let mut visited_dirs = HashSet::new();
    collect_rs(
        scope,
        sandbox,
        &mut files,
        &mut graph.diagnostics,
        &mut visited_dirs,
    )?;
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
                if let Some(message) = delimiter_diagnostic(&masked) {
                    graph.diagnostics.push(Diagnostic {
                        file: rel.clone(),
                        message,
                    });
                }
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

fn delimiter_diagnostic(masked: &str) -> Option<String> {
    let mut stack: Vec<(char, usize)> = Vec::new();
    for (idx, ch) in masked.char_indices() {
        match ch {
            '{' | '(' | '[' => stack.push((ch, idx)),
            '}' | ')' | ']' => {
                let expected = match ch {
                    '}' => '{',
                    ')' => '(',
                    ']' => '[',
                    _ => unreachable!(),
                };
                if !matches!(stack.pop(), Some((open, _)) if open == expected) {
                    return Some(format!(
                        "partial parse diagnostic: unmatched closing delimiter '{ch}' at byte {idx}; results may be incomplete"
                    ));
                }
            }
            _ => {}
        }
    }
    stack.pop().map(|(open, idx)| {
        format!(
            "partial parse diagnostic: unmatched opening delimiter '{open}' at byte {idx}; results may be incomplete"
        )
    })
}

fn collect_rs(
    path: &Path,
    sandbox: &Sandbox,
    out: &mut Vec<PathBuf>,
    diagnostics: &mut Vec<Diagnostic>,
    visited_dirs: &mut HashSet<PathBuf>,
) -> Result<(), String> {
    let validated = sandbox
        .validate_path(&path.to_string_lossy())
        .map_err(|e| e.to_string())?;
    if validated.is_file() {
        if validated.extension().and_then(|e| e.to_str()) == Some("rs") {
            push_candidate(&validated, sandbox, out, diagnostics);
        }
        return Ok(());
    }
    if !validated.exists() {
        return Err(format!(
            "failed to read {}: path does not exist",
            validated.display()
        ));
    }
    let canonical = validated
        .canonicalize()
        .map_err(|e| format!("failed to resolve {}: {e}", validated.display()))?;
    if !visited_dirs.insert(canonical) {
        diagnostics.push(Diagnostic {
            file: rel_path(&validated, &validated),
            message: "skipped already-visited directory to avoid symlink cycle".into(),
        });
        return Ok(());
    }
    let entries = std::fs::read_dir(&validated)
        .map_err(|e| format!("failed to read {}: {e}", validated.display()))?;
    for entry in entries.flatten() {
        let p = entry.path();
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if name == ".git" || name == "target" || name == ".quecto" {
            continue;
        }
        if p.is_dir() {
            collect_rs(&p, sandbox, out, diagnostics, visited_dirs)?;
        } else if p.extension().and_then(|e| e.to_str()) == Some("rs") {
            push_candidate(&p, sandbox, out, diagnostics);
        }
        if out.len() >= MAX_RUST_FILES {
            diagnostics.push(Diagnostic {
                file: rel_path(&validated, &validated),
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
    let item_re = Regex::new(r"(?m)^[ \t]*((?:pub(?:\([^)]*\))?[ \t]+)?)((?:async[ \t]+)?(?:unsafe[ \t]+)?)\b(fn|struct|enum|trait|type|const|static|mod)\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap();
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
        location: super::rust_ast_graph::Location {
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
