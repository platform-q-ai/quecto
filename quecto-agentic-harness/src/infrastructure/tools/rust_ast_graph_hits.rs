use serde::Serialize;

use super::rust_ast_graph::RustFile;
use super::rust_ast_graph_text::{line_col, snippet};

#[derive(Serialize)]
pub(super) struct UseHit {
    pub(super) file: String,
    pub(super) line: usize,
    pub(super) column: usize,
    pub(super) use_tree: String,
}

#[derive(Serialize)]
pub(super) struct RefHit {
    pub(super) file: String,
    pub(super) line: usize,
    pub(super) column: usize,
    pub(super) byte_start: usize,
    pub(super) byte_end: usize,
    pub(super) kind: &'static str,
    pub(super) snippet: String,
}

pub(super) fn hit_for(
    f: &RustFile,
    start: usize,
    end: usize,
    snippet_lines: usize,
    kind: &'static str,
) -> RefHit {
    let (line, column) = line_col(&f.text, start);
    RefHit {
        file: f.rel.clone(),
        line,
        column,
        byte_start: start,
        byte_end: end,
        kind,
        snippet: snippet(&f.text, line, snippet_lines),
    }
}
