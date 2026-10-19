//! Who may start an authority server (#1910, #2622 review).
//!
//! The start functions are `pub(crate)`, so the compiler refuses a start
//! from outside the crate. Inside it, this parse-based scan resolves every
//! path through the crate's aliases (`use … as S`, `pub use … as Broker`,
//! `type S = …`) and qualified-self paths (`<AuthorityServer>::start`), so
//! only the server itself, composition and the broker's run path start one.
//! The test-support start functions in composition stay behind their gate.

use std::collections::BTreeMap;
use std::path::Path;

use super::teardown_authority::walk;
use super::use_case_construction::{crate_raw_aliases, ident_name, syn_aliases_of};

const SERVER: &str = "AuthorityServer";

/// The files that may start an authority server: its own module, composition
/// and the `admission-broker run` path.
const STARTERS: &[&str] = &[
    "src/infrastructure/admission/server.rs",
    "src/composition/admission.rs",
    "src/interface/cli/admission_broker.rs",
];

struct Starts<'a> {
    aliases: &'a BTreeMap<String, String>,
    impl_types: Vec<String>,
    found: Vec<String>,
}

impl Starts<'_> {
    /// Whether `ident` (or `Self` in the enclosing impl) names the server,
    /// through any chain of aliases.
    fn names_server(&self, ident: &str) -> bool {
        let mut current = match ident {
            "Self" => match self.impl_types.last() {
                Some(ty) => ty.clone(),
                None => return false,
            },
            other => other.to_string(),
        };
        for _ in 0..=self.aliases.len() {
            if current == SERVER {
                return true;
            }
            match self.aliases.get(&current) {
                Some(next) if *next != current => current = next.clone(),
                _ => return false,
            }
        }
        false
    }

    fn type_names_server(&self, ty: &syn::Type) -> bool {
        match ty {
            syn::Type::Paren(ty) => self.type_names_server(&ty.elem),
            syn::Type::Group(ty) => self.type_names_server(&ty.elem),
            syn::Type::Path(ty) => match &ty.qself {
                Some(qself) => self.type_names_server(&qself.ty),
                None => ty
                    .path
                    .segments
                    .last()
                    .is_some_and(|segment| self.names_server(&ident_name(&segment.ident))),
            },
            _ => false,
        }
    }
}

impl<'ast> syn::visit::Visit<'ast> for Starts<'_> {
    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let self_ty = match &*item.self_ty {
            syn::Type::Path(ty) => ty
                .path
                .segments
                .last()
                .map(|segment| ident_name(&segment.ident)),
            _ => None,
        };
        self.impl_types.push(self_ty.unwrap_or_default());
        syn::visit::visit_item_impl(self, item);
        self.impl_types.pop();
    }

    fn visit_expr_path(&mut self, expr: &'ast syn::ExprPath) {
        let segments: Vec<String> = expr
            .path
            .segments
            .iter()
            .map(|segment| ident_name(&segment.ident))
            .collect();
        if let Some(last) = segments.last()
            && last.starts_with("start")
        {
            let owner = match &expr.qself {
                Some(qself) => self.type_names_server(&qself.ty),
                None => segments
                    .len()
                    .checked_sub(2)
                    .is_some_and(|owner| self.names_server(&segments[owner])),
            };
            if owner {
                self.found.push(segments.join("::"));
            }
        }
        syn::visit::visit_expr_path(self, expr);
    }
}

/// Every start of an authority server `source` spells, resolved through the
/// crate's aliases and the file's own.
fn starts_in(source: &str) -> Vec<String> {
    use syn::visit::Visit;
    let Ok(file) = syn::parse_file(source) else {
        // Fail closed: a file the scan cannot read is a starter if it names
        // the server at all.
        return if source.contains(SERVER) {
            vec!["<does not parse>".to_string()]
        } else {
            Vec::new()
        };
    };
    let mut aliases = crate_raw_aliases().clone();
    aliases.extend(syn_aliases_of(&file));
    let mut scan = Starts {
        aliases: &aliases,
        impl_types: Vec::new(),
        found: Vec::new(),
    };
    scan.visit_file(&file);
    scan.found
}

/// Every authority server, in production or under test, is started the way
/// composition composes it: tests through `composition::admission`'s
/// test-support start functions, the broker through its run path. No other
/// file, test files included, starts one, however the path is spelled.
#[test]
fn only_composition_and_the_broker_start_an_authority_server() {
    let mut files = Vec::new();
    walk(Path::new("src"), &mut files);
    walk(Path::new("tests"), &mut files);
    let mut outside = Vec::new();
    let mut allowed = 0usize;
    for file in files {
        let found = starts_in(&std::fs::read_to_string(&file).unwrap());
        if STARTERS.contains(&file.as_str()) {
            allowed += found.len();
        } else {
            outside.extend(found.into_iter().map(|start| format!("{file}: {start}")));
        }
    }
    assert!(
        outside.is_empty(),
        "an authority server is started only through composition or the broker's run path:\n{}",
        outside.join("\n")
    );
    assert!(allowed >= 5, "the allowed starts were seen ({allowed})");
}

/// The scan itself: every spelling of a start is caught, and other types'
/// `start` and the server's other functions are spared.
#[test]
fn the_start_scan_resolves_aliases_and_qualified_self() {
    let caught = [
        "fn f() { AuthorityServer::start(d, p); }",
        "fn f() { crate::infrastructure::admission::AuthorityServer::start_accepting_missing_ledger(d, p); }",
        "use crate::infrastructure::admission::AuthorityServer as S;\nfn f() { S::start(d, p); }",
        "fn f() { <crate::infrastructure::admission::AuthorityServer>::start(d, p); }",
        "fn f() { <AuthorityServer as Starter>::start(d, p); }",
        "pub use crate::infrastructure::admission::AuthorityServer as Broker;\nfn f() { Broker::start(d, p); }",
        "type S = crate::infrastructure::admission::AuthorityServer;\nfn f() { S::start_with_frame_deadline(d, p, t); }",
        "impl AuthorityServer { fn again() { Self::start(d, p); } }",
        "fn f() { let start = AuthorityServer::start; }",
        "fn f() { r#AuthorityServer::start(d, p); }",
    ];
    for source in caught {
        assert!(!starts_in(source).is_empty(), "must catch:\n{source}");
    }
    let spared = [
        "fn f() { Other::start(d, p); }",
        "fn f(s: &AuthorityServer) { s.directory(); }",
        "fn f() { AuthorityServer::directory(s); }",
        "impl Other { fn again() { Self::start(d, p); } }",
        "fn f() { start_authority(d, p); }",
    ];
    for source in spared {
        let found = starts_in(source);
        assert!(found.is_empty(), "must spare:\n{source}\nfound {found:?}");
    }
}

/// Composition's start functions exist only in test and `test-support`
/// builds: production starts the broker through its run path alone.
#[test]
fn composition_start_functions_are_test_support_only() {
    let source = std::fs::read_to_string("src/composition/admission.rs").unwrap();
    let file = syn::parse_file(&source).expect("composition parses");
    let starts: Vec<&syn::ItemFn> = file
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Fn(function) => Some(function),
            _ => None,
        })
        .filter(|function| {
            function
                .sig
                .ident
                .to_string()
                .starts_with("start_authority")
        })
        .collect();
    assert_eq!(starts.len(), 3, "the three start functions are declared");
    for function in starts {
        assert!(
            super::cfg_test_gated(&function.attrs),
            "`{}` is gated to test and test-support builds",
            function.sig.ident
        );
    }
}
