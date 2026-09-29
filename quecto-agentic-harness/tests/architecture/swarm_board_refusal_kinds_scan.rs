//! The refusal-site scan: every `BoardError::new(kind, text)` a source
//! builds, read as the table names it (#2303), and its own tests, which
//! count production sources only.
use quote::ToTokens;
use syn::visit::Visit;

use super::production;

/// One `BoardError::new(kind, text)` site: the function it is in
/// (`Type::method` inside an impl), its text and its kind.
type Site = (String, String, String);

/// Every `BoardError::new(kind, text)` call, with the function it is in.
#[derive(Default)]
struct Built {
    sites: Vec<Site>,
    /// The impl's self type and the functions entered, innermost last.
    impls: Vec<String>,
    functions: Vec<String>,
}

impl Built {
    fn function(&self) -> String {
        let function = self.functions.last().map_or("<item>", String::as_str);
        match self.impls.last() {
            Some(owner) if self.functions.len() == 1 => format!("{owner}::{function}"),
            _ => function.to_owned(),
        }
    }
}

fn names_constructor(path: &syn::Path) -> bool {
    let segments: Vec<String> = path.segments.iter().map(|s| s.ident.to_string()).collect();
    segments.ends_with(&["BoardError".to_owned(), "new".to_owned()])
}

fn tokens(expr: &syn::Expr) -> String {
    tokens_of(expr)
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
    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let owner = match &*item.self_ty {
            syn::Type::Path(path) => path
                .path
                .segments
                .last()
                .map_or_else(|| tokens_of(&item.self_ty), |s| s.ident.to_string()),
            other => tokens_of(other),
        };
        self.impls.push(owner);
        syn::visit::visit_item_impl(self, item);
        self.impls.pop();
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.functions.push(item.sig.ident.to_string());
        let impls = std::mem::take(&mut self.impls);
        syn::visit::visit_item_fn(self, item);
        self.impls = impls;
        self.functions.pop();
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.functions.push(item.sig.ident.to_string());
        syn::visit::visit_impl_item_fn(self, item);
        self.functions.pop();
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if let syn::Expr::Path(function) = &*call.func {
            if names_constructor(&function.path) {
                let arguments: Vec<&syn::Expr> = call.args.iter().collect();
                assert_eq!(arguments.len(), 2, "BoardError::new(kind, text)");
                let site = (
                    self.function(),
                    text_of(arguments[1]),
                    kind_of(arguments[0]),
                );
                self.sites.push(site);
            }
        }
        syn::visit::visit_expr_call(self, call);
    }
}

fn tokens_of(tokens: &impl ToTokens) -> String {
    tokens
        .to_token_stream()
        .to_string()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn built(source: &str) -> Vec<Site> {
    let file = syn::parse_file(source).expect("a crate source parses");
    let mut found = Built::default();
    found.visit_file(&file);
    found.sites
}

#[test]
fn the_scan_reads_texts_templates_and_kinds() {
    let found = built(
        r#"fn f() {
            let _ = BoardError::new(RefusalKind::Invalid, "plain");
            let _ = BoardError::new(RefusalKind::NotRunning, format!("run is {}; no", x));
            let _ = crate::domain::swarm::BoardError::new(kind, error.to_string());
            let _ = StoreRefusal(m);
        }
        impl Store {
            fn g(&self) {
                let _ = || BoardError::new(RefusalKind::Invalid, "plain");
                fn inner() { let _ = BoardError::new(RefusalKind::Invalid, "plain"); }
            }
        }"#,
    );
    let site = |function: &str, text: &str, kind: &str| {
        (function.to_owned(), text.to_owned(), kind.to_owned())
    };
    assert_eq!(
        found,
        [
            site("f", "plain", "Invalid"),
            site("f", "run is {}; no", "NotRunning"),
            site("f", "expr: error . to_string ()", "expr: kind"),
            site("Store::g", "plain", "Invalid"),
            site("inner", "plain", "Invalid"),
        ]
    );
    assert!(production("src/domain/swarm/policy.rs"));
    assert!(!production("src/domain/swarm/telemetry_tests.rs"));
    assert!(!production("src/application/swarm/board_test_support.rs"));
    // An allowlist (#2303 round-3 review L5): a file the production module
    // tree does not mount is not production, whatever its name.
    assert!(!production("src/nowhere.rs"));
    assert!(!production(
        "src/application/swarm/use_cases/create_run_tests.rs"
    ));
}
