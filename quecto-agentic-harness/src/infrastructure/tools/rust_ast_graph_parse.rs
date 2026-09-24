use std::collections::HashSet;
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::infrastructure::security::sandbox::Sandbox;

use super::rust_ast_graph::{Diagnostic, Graph, RustFile, Symbol, SymbolParts};
use super::rust_ast_graph_text::{
    line_col, line_end, mask_comments_and_strings, module_path, rel_path, snippet,
};

pub(super) const MAX_RUST_FILES: usize = 2_000;
pub(super) const MAX_TOTAL_BYTES: u64 = 25 * 1024 * 1024;
pub(super) const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
pub(super) const MAX_VISITED_DIRS: usize = 5_000;
pub(super) const MAX_SCANNED_ENTRIES: usize = 50_000;

#[derive(Default)]
struct TraversalBudget {
    visited_dirs: usize,
    scanned_entries: usize,
    exhausted: bool,
    file_limit_reported: bool,
}

struct CollectContext<'a> {
    workspace: &'a Path,
    sandbox: &'a Sandbox,
    workspace_root: &'a Path,
    out: &'a mut Vec<PathBuf>,
    diagnostics: &'a mut Vec<Diagnostic>,
    visited_dirs: &'a mut HashSet<PathBuf>,
    budget: &'a mut TraversalBudget,
}

pub(super) fn build_graph(
    scope: &Path,
    workspace: &Path,
    sandbox: &Sandbox,
    snippet_lines: usize,
) -> Result<Graph, String> {
    let workspace_root = workspace
        .canonicalize()
        .map_err(|e| format!("failed to resolve workspace: {e}"))?;
    let scope_root = scope
        .canonicalize()
        .map_err(|e| format!("failed to read scope: {e}"))?;
    if !scope_root.starts_with(&workspace_root) {
        return Err("scope is outside workspace".into());
    }
    let mut graph = Graph {
        workspace: workspace.to_path_buf(),
        ..Graph::default()
    };
    let mut files = Vec::new();
    let mut visited_dirs = HashSet::new();
    let mut budget = TraversalBudget::default();
    let mut ctx = CollectContext {
        workspace,
        sandbox,
        workspace_root: &workspace_root,
        out: &mut files,
        diagnostics: &mut graph.diagnostics,
        visited_dirs: &mut visited_dirs,
        budget: &mut budget,
    };
    collect_rs(scope, &mut ctx)?;
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
        if !validated
            .canonicalize()
            .is_ok_and(|p| p.starts_with(&workspace_root))
        {
            graph.diagnostics.push(Diagnostic {
                file: rel,
                message: "skipped path outside workspace".into(),
            });
            continue;
        }
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

fn collect_rs(path: &Path, ctx: &mut CollectContext<'_>) -> Result<(), String> {
    let validated = ctx
        .sandbox
        .validate_path(&path.to_string_lossy())
        .map_err(|e| e.to_string())?;
    if !validated
        .canonicalize()
        .is_ok_and(|p| p.starts_with(ctx.workspace_root))
    {
        return Err("path outside workspace".into());
    }
    if validated.is_file() {
        if validated.extension().and_then(|e| e.to_str()) == Some("rs") {
            push_candidate(
                &validated,
                ctx.sandbox,
                ctx.workspace_root,
                ctx.out,
                ctx.diagnostics,
            );
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
    if !ctx.visited_dirs.insert(canonical) {
        ctx.diagnostics.push(Diagnostic {
            file: rel_path(&validated, ctx.workspace),
            message: "skipped already-visited directory to avoid symlink cycle".into(),
        });
        return Ok(());
    }
    ctx.budget.visited_dirs += 1;
    if ctx.budget.visited_dirs > MAX_VISITED_DIRS {
        if !ctx.budget.exhausted {
            ctx.diagnostics.push(Diagnostic {
                file: rel_path(&validated, ctx.workspace),
                message: format!("stopped after MAX_VISITED_DIRS={MAX_VISITED_DIRS}"),
            });
            ctx.budget.exhausted = true;
        }
        return Ok(());
    }
    let entries = std::fs::read_dir(&validated)
        .map_err(|e| format!("failed to read {}: {e}", validated.display()))?;
    for entry in entries.flatten() {
        if ctx.out.len() >= MAX_RUST_FILES {
            report_file_limit(&validated, ctx);
            break;
        }
        if ctx.budget.scanned_entries >= MAX_SCANNED_ENTRIES {
            if !ctx.budget.exhausted {
                ctx.diagnostics.push(Diagnostic {
                    file: rel_path(&validated, ctx.workspace),
                    message: format!("stopped after MAX_SCANNED_ENTRIES={MAX_SCANNED_ENTRIES}"),
                });
                ctx.budget.exhausted = true;
            }
            break;
        }
        ctx.budget.scanned_entries += 1;
        let p = entry.path();
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if is_skipped_dir(name) {
            continue;
        }
        if p.is_dir() {
            if ctx.out.len() >= MAX_RUST_FILES {
                report_file_limit(&validated, ctx);
                break;
            }
            if let Err(e) = collect_rs(&p, ctx) {
                ctx.diagnostics.push(Diagnostic {
                    file: rel_path(&p, ctx.workspace),
                    message: format!("skipped directory: {e}"),
                });
            }
        } else if p.extension().and_then(|e| e.to_str()) == Some("rs") {
            push_candidate(
                &p,
                ctx.sandbox,
                ctx.workspace_root,
                ctx.out,
                ctx.diagnostics,
            );
        }
        if ctx.out.len() >= MAX_RUST_FILES {
            report_file_limit(&validated, ctx);
            break;
        }
    }
    Ok(())
}

fn report_file_limit(dir: &Path, ctx: &mut CollectContext<'_>) {
    if !ctx.budget.file_limit_reported {
        ctx.diagnostics.push(Diagnostic {
            file: rel_path(dir, ctx.workspace),
            message: format!("stopped after MAX_RUST_FILES={MAX_RUST_FILES}"),
        });
        ctx.budget.file_limit_reported = true;
    }
}

fn is_skipped_dir(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | "target"
            | ".quecto"
            | "node_modules"
            | "vendor"
            | "dist"
            | "build"
            | ".next"
            | ".cache"
    )
}

fn push_candidate(
    path: &Path,
    sandbox: &Sandbox,
    workspace_root: &Path,
    out: &mut Vec<PathBuf>,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if out.len() >= MAX_RUST_FILES {
        return;
    }
    match sandbox.validate_path(&path.to_string_lossy()) {
        Ok(validated)
            if validated
                .canonicalize()
                .is_ok_and(|p| p.starts_with(workspace_root)) =>
        {
            out.push(validated)
        }
        Ok(_) => diagnostics.push(Diagnostic {
            file: path.to_string_lossy().into_owned(),
            message: "skipped path outside workspace".into(),
        }),
        Err(e) => diagnostics.push(Diagnostic {
            file: path.to_string_lossy().into_owned(),
            message: format!("skipped path outside sandbox: {e}"),
        }),
    }
}

fn parse_symbols(file: &RustFile, snippet_lines: usize) -> Vec<Symbol> {
    // Qualifiers are an allowlist: `const` before `fn` is not a `const` item.
    let item_re = Regex::new(r"(?m)(?:^|[;{}])[ \t]*((?:pub(?:\([^)]*\))?[ \t]+)?)(?:(?:async|unsafe|const)[ \t]+)*(fn|struct|enum|trait|type|const|static|mod)[ \t]+([\p{XID_Start}_][\p{XID_Continue}_]*)").unwrap();
    let impl_re = Regex::new(r"(?m)(?:^|[;{}])[ \t]*(?:unsafe[ \t]+)?impl\b").unwrap();
    let mut declarations = Vec::new();
    for cap in item_re.captures_iter(&file.masked) {
        let keyword = cap.get(2).unwrap();
        let name = cap.get(3).unwrap();
        let full = cap.get(0).unwrap();
        let prefix_len = full
            .as_str()
            .bytes()
            .take_while(|byte| matches!(byte, b'{' | b'}' | b';' | b' ' | b'\t' | b'\n'))
            .count();
        let start = full.start() + prefix_len;
        // The delimiter belongs to the previous scope, never the declaration.
        let visibility = cap
            .get(1)
            .map(|v| v.as_str().trim())
            .filter(|v| !v.is_empty())
            .unwrap_or("private");
        declarations.push(make_symbol(SymbolParts {
            file,
            name: name.as_str(),
            kind: keyword.as_str(),
            visibility,
            start,
            end: name.end(),
            trait_name: None,
            for_type: None,
            snippet_lines,
        }));
    }
    for m in impl_re.find_iter(&file.masked) {
        let keyword_end = m.end();
        let Some((target, end)) = impl_target(&file.masked, keyword_end) else {
            continue;
        };
        let (trait_name, for_type, name) = if let Some((tr, ty)) = split_impl_target(target) {
            (
                Some(tr.to_string()),
                Some(ty.to_string()),
                format!("impl {tr} for {ty}"),
            )
        } else {
            (None, Some(target.to_string()), format!("impl {target}"))
        };
        let start = m
            .as_str()
            .find("impl")
            .map_or(m.start(), |offset| m.start() + offset);
        declarations.push(make_symbol(SymbolParts {
            file,
            name: &name,
            kind: "impl",
            visibility: "inherent/syntactic",
            start,
            end,
            trait_name,
            for_type,
            snippet_lines,
        }));
    }
    // Walk real braces in the masked text: comments and string literals cannot
    // open a scope. Only a recognized module/impl/trait declaration contributes
    // an ownership component; function/control-flow braces are transparent.
    declarations.sort_by_key(|symbol| symbol.location.byte_start);
    let mut syms = Vec::with_capacity(declarations.len());
    let mut braces: Vec<Option<String>> = Vec::new();
    let mut cursor = 0;
    let root = if file.module == "crate" {
        String::new()
    } else {
        file.module.clone()
    };
    for mut symbol in declarations {
        let start = symbol.location.byte_start;
        if start < cursor {
            continue;
        }
        for ch in file.masked[cursor..start].chars() {
            match ch {
                '{' => braces.push(None),
                '}' => {
                    braces.pop();
                }
                _ => {}
            }
        }
        let mut module = root.clone();
        for segment in braces.iter().flatten() {
            if !module.is_empty() {
                module.push_str("::");
            }
            module.push_str(segment);
        }
        symbol.module = if module.is_empty() {
            "crate".into()
        } else {
            module.clone()
        };
        symbol.qualified_path = if module.is_empty() {
            symbol.name.clone()
        } else {
            format!("{module}::{}", symbol.name)
        };
        let header_end = symbol.location.byte_end;
        let body = if symbol.kind == "impl" {
            Some(header_end - 1)
        } else if matches!(symbol.kind.as_str(), "mod" | "trait") {
            file.masked[header_end..]
                .char_indices()
                .find(|(_, ch)| matches!(ch, '{' | ';'))
                .and_then(|(i, ch)| (ch == '{').then_some(header_end + i))
        } else {
            None
        };
        // Advance through the header and its opening brace, preserving nested
        // angle/parenthesis content as text rather than scope transitions.
        let end = body.map_or(header_end, |open| open + 1);
        for (offset, ch) in file.masked[start..end].char_indices() {
            match ch {
                '{' => {
                    let owner = if Some(start + offset) == body {
                        match symbol.kind.as_str() {
                            "mod" | "trait" => Some(symbol.name.clone()),
                            "impl" => symbol.for_type.as_ref().map(|ty| ty.trim().to_string()),
                            _ => None,
                        }
                    } else {
                        None
                    };
                    braces.push(owner);
                }
                '}' => {
                    braces.pop();
                }
                _ => {}
            }
        }
        cursor = end;
        syms.push(symbol);
    }
    syms
}

// Walk an impl header rather than matching a single line: a `where` clause may
// contain arbitrary newlines and generic parameter bounds may contain colons.
// Only delimiters at the header level may end the header.
fn impl_target(masked: &str, after_impl: usize) -> Option<(&str, usize)> {
    let tail = &masked[after_impl..];
    let mut angle_depth = 0usize;
    let mut braces = None;
    let mut where_start = None;
    for (offset, ch) in tail.char_indices() {
        match ch {
            '<' => angle_depth += 1,
            '>' => angle_depth = angle_depth.saturating_sub(1),
            '{' if angle_depth == 0 => {
                braces = Some(offset);
                break;
            }
            ';' if angle_depth == 0 => return None,
            'w' if angle_depth == 0 && where_start.is_none() => {
                let remaining = &tail[offset..];
                if remaining.starts_with("where")
                    && tail[..offset]
                        .chars()
                        .last()
                        .is_some_and(char::is_whitespace)
                    && remaining[5..]
                        .chars()
                        .next()
                        .is_some_and(char::is_whitespace)
                {
                    where_start = Some(offset);
                }
            }
            _ => {}
        }
    }
    let brace = braces?;
    let header = tail[..where_start.unwrap_or(brace).min(brace)].trim();
    // Generics following `impl` describe parameters, not the target type.
    let mut rest = header;
    if let Some(params) = rest.strip_prefix('<') {
        let mut depth = 1usize;
        let mut end = None;
        for (offset, ch) in params.char_indices() {
            match ch {
                '<' => depth += 1,
                '>' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(offset + ch.len_utf8());
                        break;
                    }
                }
                _ => {}
            }
        }
        rest = params[end?..].trim();
    }
    (!rest.is_empty()).then_some((rest, after_impl + brace + 1))
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

#[cfg(test)]
mod parser_review_tests {
    use super::*;

    fn symbols(text: &str) -> Vec<Symbol> {
        let file = RustFile {
            rel: "src/lib.rs".into(),
            module: "crate".into(),
            text: text.into(),
            masked: mask_comments_and_strings(text),
        };
        parse_symbols(&file, 0)
    }

    #[test]
    fn fn_qualifiers_are_parsed_without_fictitious_fn_identifier() {
        let syms = symbols(
            "pub const fn make() {}\nunsafe async fn boom() {}\nasync unsafe fn mixed() {}\n",
        );
        for name in ["make", "boom", "mixed"] {
            assert!(
                syms.iter().any(|s| s.name == name && s.kind == "fn"),
                "missing {name}"
            );
        }
        assert!(!syms.iter().any(|s| s.name == "fn"));
    }

    #[test]
    fn impl_headers_discard_where_bounds_and_handle_multiline_trait_impls() {
        let syms = symbols(
            "impl<T> Widget<T> where T: Clone { }\nimpl<T> Build for Widget<T>\nwhere T: Clone,\n{ }\nimpl<T: Clone> Build for Other<T> { }\n",
        );
        assert!(syms.iter().any(|s| s.kind == "impl"
            && s.trait_name.is_none()
            && s.for_type.as_deref() == Some("Widget<T>")));
        assert!(syms.iter().any(|s| s.kind == "impl"
            && s.trait_name.as_deref() == Some("Build")
            && s.for_type.as_deref() == Some("Widget<T>")));
        assert!(syms.iter().any(|s| s.kind == "impl"
            && s.trait_name.as_deref() == Some("Build")
            && s.for_type.as_deref() == Some("Other<T>")));
    }
}

#[cfg(all(test, unix))]
mod workspace_boundary_tests {
    use super::*;

    #[test]
    fn symlinks_outside_workspace_are_not_indexed_even_when_sandbox_allows_them() {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let outside = root.path().join("outside");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(workspace.join("inside.rs"), "fn inside() {}").unwrap();
        std::fs::write(outside.join("outside.rs"), "fn escaped() {}").unwrap();
        std::os::unix::fs::symlink(outside.join("outside.rs"), workspace.join("file_link.rs"))
            .unwrap();
        std::os::unix::fs::symlink(&outside, workspace.join("dir_link")).unwrap();
        let sandbox = Sandbox::new(Some(root.path().to_path_buf()));
        let graph = build_graph(&workspace, &workspace, &sandbox, 0).unwrap();
        assert!(graph.symbols.iter().any(|s| s.name == "inside"));
        assert!(!graph.symbols.iter().any(|s| s.name == "escaped"));
        assert!(
            graph
                .diagnostics
                .iter()
                .any(|d| d.message.contains("outside workspace"))
        );
        assert!(build_graph(&outside, &workspace, &sandbox, 0).is_err());
    }
}
