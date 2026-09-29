//! Every board refusal carries a kind (#2303): a `BoardError` is built
//! only through `BoardError::new(kind, message)`, whose kind the
//! `swarm_op` telemetry records. Its fields are private, so outside its
//! own module a tuple or struct literal does not compile; this finds one
//! anywhere else in the crate's sources and tests, so a refusal can never
//! bypass the mapping.
use std::path::Path;

use syn::visit::Visit;

/// The only file that builds `BoardError` from its fields.
const DEFINITION: &str = "src/domain/swarm/mod.rs";

/// `BoardError(…)` and `BoardError { … }` literals, by line.
#[derive(Default)]
struct Literals(Vec<usize>);

fn names_board_error(path: &syn::Path) -> bool {
    path.segments
        .last()
        .is_some_and(|segment| segment.ident == "BoardError")
}

impl<'ast> Visit<'ast> for Literals {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(function) = &*call.func {
            if names_board_error(&function.path) {
                self.0.push(syn::spanned::Spanned::span(call).start().line);
            }
        }
        syn::visit::visit_expr_call(self, call);
    }

    fn visit_expr_struct(&mut self, literal: &'ast syn::ExprStruct) {
        if names_board_error(&literal.path) {
            self.0
                .push(syn::spanned::Spanned::span(literal).start().line);
        }
        syn::visit::visit_expr_struct(self, literal);
    }
}

fn literals(source: &str) -> Vec<usize> {
    let file = syn::parse_file(source).expect("a crate source parses");
    let mut found = Literals::default();
    found.visit_file(&file);
    found.0
}

fn sources(dir: &Path, files: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            sources(&path, files);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let content = std::fs::read_to_string(&path).expect("read file");
            files.push((path.display().to_string(), content));
        }
    }
}

#[test]
fn every_board_refusal_is_built_with_its_kind() {
    let mut files = Vec::new();
    sources(Path::new("src"), &mut files);
    sources(Path::new("tests"), &mut files);
    assert!(
        files.iter().any(|(path, _)| path == DEFINITION),
        "the scan covers the definition"
    );
    let bypasses: Vec<String> = files
        .iter()
        .filter(|(path, _)| path != DEFINITION)
        .flat_map(|(path, content)| {
            literals(content)
                .into_iter()
                .map(move |line| format!("{path}:{line}"))
        })
        .collect();
    assert!(
        bypasses.is_empty(),
        "build a BoardError with BoardError::new(kind, message): {bypasses:?}"
    );
}

#[test]
fn the_scan_finds_every_literal_and_passes_the_constructor() {
    assert_eq!(literals("fn f() { let _ = BoardError(m); }"), [1]);
    assert_eq!(
        literals("fn f() { let _ = crate::domain::swarm::BoardError(m); }"),
        [1]
    );
    assert_eq!(
        literals("fn f() {\n let _ = BoardError { kind, message };\n}"),
        [2]
    );
    assert_eq!(
        literals("fn f() { x.map_err(|e| Err(BoardError(e.0))); }"),
        [1]
    );
    assert!(
        literals("fn f() { let _ = BoardError::new(RefusalKind::Invalid, \"x\"); }").is_empty()
    );
    assert!(literals("fn f() { let _ = StoreRefusal(m); }").is_empty());
}
