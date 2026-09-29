//! Every board refusal text maps to exactly one kind (#2303, review L1):
//! a table of each `BoardError::new(kind, text)` the production sources
//! build, by text, names the one [`RefusalKind`] telemetry records for it.
//! A refusal whose kind changes (to a wrong one, or to a catch-all such as
//! `Internal`), a new refusal, a text raised under two kinds, and a table
//! row no source builds any more all fail here, so each kind is chosen and
//! reviewed against its text once.
//!
//! A text is the message's string literal, or the template of its
//! `format!`; a message built some other way (a store error's own text, a
//! codec's) is its expression, prefixed `expr:`. A kind is the
//! `RefusalKind` variant named, or, for one chosen at run time, the
//! expression that chooses it, prefixed `expr:`; those choosers are tested
//! where they are defined (`policy.rs`'s budget kind,
//! `repository.rs`'s store kinds).
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use quote::ToTokens;
use syn::visit::Visit;

/// `(text, kind)` for every refusal the production sources build.
const REFUSALS: &[(&str, &str)] = &[
    (
        "completion requires accepted evidence at the current revision for every criterion",
        "CompletionUnmet",
    ),
    ("completion revision required", "Invalid"),
    ("constraints must be a list of strings", "Invalid"),
    ("coordination run missing", "RunMissing"),
    (
        "coordination store committed without running its work",
        "Internal",
    ),
    (
        "coordination store ran a transaction's work twice",
        "Internal",
    ),
    (
        "criteria distinguish command checks from parent-reviewed requirements",
        "Invalid",
    ),
    ("deadline extension must be 1..604800 seconds", "Invalid"),
    ("deadline must be in the next seven days", "Invalid"),
    ("duplicate criterion id", "Invalid"),
    (
        "existing live/reserved members exceed requested limit; terminate and reconcile first",
        "MemberLimit",
    ),
    ("explicit evidence criteria required", "Invalid"),
    ("expr: contended (& error) . 0", "expr: kind"),
    ("expr: error", "Invalid"),
    ("expr: error . to_string ()", "Invalid"),
    ("expr: message", "Invalid"),
    ("expr: refusal . 0", "expr: store_kind (failure)"),
    ("invoking member is unknown or death confirmed", "NotMember"),
    (
        "member identity already used; choose a stable new identity",
        "IdentityTaken",
    ),
    (
        "member limit must be 1 through 25 including coordinator",
        "Invalid",
    ),
    ("new artifact and revision evidence required", "Invalid"),
    (
        "new artifact evidence must match the revalidated revision",
        "Invalid",
    ),
    ("only completed tasks may be revalidated", "WrongState"),
    (
        "only the designated coordinator may do this",
        "NotCoordinator",
    ),
    (
        "only the setup coordinator can create this run; existing runs cannot be reset",
        "RunExists",
    ),
    (
        "run is paused (budget-exhausted: deadline); no new work permitted",
        "BudgetExhausted",
    ),
    (
        "run is {}; no new admission",
        "expr: not_running (budget_spent (run) || expired (run , now))",
    ),
    (
        "run is {}; no new work permitted",
        "expr: not_running (budget_spent (run))",
    ),
    (
        "settle outstanding work and file reservations before success",
        "CompletionUnmet",
    ),
    (
        "submitted evidence is immutable; release and reclaim before revising",
        "Immutable",
    ),
    (
        "swarm board bound the wrong number of arguments",
        "Internal",
    ),
    ("swarm board has no method {method}", "Calling"),
    ("swarm board meter did not run the call", "Internal"),
    (
        "swarm limit {}, current usage {usage}; reuse the existing pool",
        "MemberLimit",
    ),
    ("task evidence refers to stale revision", "StaleRevision"),
    (
        "task status '{unknown}' is not a known status; no revision permitted",
        "WrongState",
    ),
    ("the board holds a non-finite number: {value}", "Store"),
    (
        "{CONTENDED}: Error binding parameter {position}: {error}",
        "Invalid",
    ),
    (
        "{label} must be nonempty and at most {maximum} bytes",
        "Invalid",
    ),
    (
        "{name}: arguments must be a JSON array or object",
        "Calling",
    ),
    ("{name}: missing required argument {}", "Calling"),
    ("{name}: takes {} arguments, {} given", "Calling"),
    ("{name}: unexpected argument {key}", "Calling"),
];

/// The files a refusal table entry may come from: production sources only
/// (a test builds refusals of any kind it likes).
fn production(path: &str) -> bool {
    let test_only = path.ends_with("_tests.rs")
        || path.ends_with("/tests.rs")
        || path.contains("/tests/")
        || path.ends_with("board_test_support.rs");
    path.starts_with("src/") && !test_only
}

/// The `(text, kind)` of every `BoardError::new(kind, text)` call.
#[derive(Default)]
struct Built(Vec<(String, String)>);

fn names_constructor(path: &syn::Path) -> bool {
    let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    segments.ends_with(&["BoardError".to_owned(), "new".to_owned()])
}

fn tokens(expr: &syn::Expr) -> String {
    expr.to_token_stream()
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn kind_of(expr: &syn::Expr) -> String {
    match expr {
        syn::Expr::Path(path)
            if path.path.segments.len() == 2 && path.path.segments[0].ident == "RefusalKind" =>
        {
            path.path.segments[1].ident.to_string()
        }
        other => format!("expr: {}", tokens(other)),
    }
}

fn text_of(expr: &syn::Expr) -> String {
    match expr {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(text),
            ..
        }) => text.value(),
        syn::Expr::Macro(call) if call.mac.path.is_ident("format") => {
            let template = call
                .mac
                .parse_body_with(
                    syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated,
                )
                .ok()
                .and_then(|arguments| arguments.first().cloned());
            match template {
                Some(syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(text),
                    ..
                })) => text.value(),
                _ => format!("expr: {}", tokens(expr)),
            }
        }
        other => format!("expr: {}", tokens(other)),
    }
}

impl<'ast> Visit<'ast> for Built {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(function) = &*call.func {
            if names_constructor(&function.path) {
                let arguments: Vec<&syn::Expr> = call.args.iter().collect();
                assert_eq!(arguments.len(), 2, "BoardError::new(kind, text)");
                self.0.push((text_of(arguments[1]), kind_of(arguments[0])));
            }
        }
        syn::visit::visit_expr_call(self, call);
    }
}

fn built(source: &str) -> Vec<(String, String)> {
    let file = syn::parse_file(source).expect("a crate source parses");
    let mut found = Built::default();
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

/// Every text the sources build, with each kind it is built under.
fn built_in_sources() -> BTreeMap<String, BTreeSet<String>> {
    let mut files = Vec::new();
    sources(Path::new("src"), &mut files);
    let mut refusals: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (path, content) in files.iter().filter(|(path, _)| production(path)) {
        for (text, kind) in built(content) {
            assert!(!kind.is_empty(), "{path}: a refusal without a kind");
            refusals.entry(text).or_default().insert(kind);
        }
    }
    refusals
}

#[test]
fn every_board_refusal_text_maps_to_exactly_one_kind() {
    let found = built_in_sources();
    assert!(found.len() > 30, "the scan reads the board's refusals");
    let two_kinds: Vec<_> = found.iter().filter(|(_, kinds)| kinds.len() > 1).collect();
    assert!(
        two_kinds.is_empty(),
        "a refusal text raised under two kinds: {two_kinds:?}"
    );
    let mut table: BTreeMap<String, String> = BTreeMap::new();
    for (text, kind) in REFUSALS {
        assert!(
            table
                .insert((*text).to_owned(), (*kind).to_owned())
                .is_none(),
            "{text} is in the table twice"
        );
    }
    let found: BTreeMap<String, String> = found
        .into_iter()
        .map(|(text, kinds)| (text, kinds.into_iter().next().unwrap()))
        .collect();
    let rows: Vec<String> = found
        .iter()
        .map(|(text, kind)| format!("    ({text:?}, {kind:?}),"))
        .collect();
    assert_eq!(
        found,
        table,
        "the refusal table is out of date; the sources build:\n{}",
        rows.join("\n")
    );
}

#[test]
fn the_scan_reads_texts_templates_and_kinds() {
    let found = built(
        r#"fn f() {
            let _ = BoardError::new(RefusalKind::Invalid, "plain");
            let _ = BoardError::new(RefusalKind::NotRunning, format!("run is {}; no", x));
            let _ = crate::domain::swarm::BoardError::new(kind, error.to_string());
            let _ = StoreRefusal(m);
        }"#,
    );
    assert_eq!(
        found,
        [
            ("plain".to_owned(), "Invalid".to_owned()),
            ("run is {}; no".to_owned(), "NotRunning".to_owned()),
            (
                "expr: error . to_string ()".to_owned(),
                "expr: kind".to_owned()
            ),
        ]
    );
    assert!(production("src/domain/swarm/policy.rs"));
    assert!(!production("src/domain/swarm/telemetry_tests.rs"));
    assert!(!production("src/application/swarm/board_test_support.rs"));
}
