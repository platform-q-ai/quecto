//! #1942 ledger rows 7, 11, 13 and 14: bounded structural proofs for
//! web_fetch, paired with the adapter's execute-path tests (these prove
//! shapes, not runtime behaviour).
//!
//! - The application use case names only its own items, the standard
//!   library, the approved `url` crate and the crate's pure domain: in its
//!   imports, in every path it writes, and inside every macro's tokens.
//! - The adapter names only allowlisted `reqwest` paths (never through a
//!   `use`), calls only allowlisted methods on the chains that build and
//!   drive its client, builds that client exactly once from the owned
//!   recipe with `no_proxy` as the very last setting, and no public
//!   function takes or returns a `reqwest::ClientBuilder`.
//!
//! What syntax cannot see (a type-inferred `Default::default()` or a
//! qualified `<reqwest::Client as Default>::default()`) is left to the
//! execute-path tests, which prove every fetch goes through the checked
//! resolver and never a proxy.
use proc_macro2::{TokenStream, TokenTree};
use std::collections::BTreeSet;
use std::fs;
use syn::visit::Visit;

const USE_CASE: &str = "src/application/agent_turn/use_cases/web_fetch.rs";
const ADAPTER: &str = "src/infrastructure/http/web_fetch.rs";

/// Roots every file may name: the standard library and `Self`. `self` and
/// `super` are not roots: they could reach anything.
const STD_ROOTS: &[&str] = &["std", "core", "alloc", "Self"];
/// The standard prelude's multi-segment roots.
const PRELUDE: &[&str] = &[
    "Option", "Some", "None", "Result", "Ok", "Err", "String", "Vec", "Box", "u8", "u16", "u32",
    "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32", "f64", "char",
    "str", "bool",
];
/// The one external crate the use case may name.
const APPROVED_CRATES: &[&str] = &["url"];

/// Every path spelled inside `tokens` (a macro's body): each run of
/// identifiers joined by `::`, with whether it starts with `::`.
fn token_paths(tokens: TokenStream, paths: &mut Vec<(bool, Vec<String>)>) {
    let tokens: Vec<TokenTree> = tokens.into_iter().collect();
    let is_colons = |at: usize| {
        matches!((tokens.get(at), tokens.get(at + 1)),
            (Some(TokenTree::Punct(a)), Some(TokenTree::Punct(b)))
                if a.as_char() == ':' && b.as_char() == ':')
    };
    let mut at = 0;
    while at < tokens.len() {
        match &tokens[at] {
            TokenTree::Group(group) => {
                token_paths(group.stream(), paths);
                at += 1;
            }
            TokenTree::Ident(_) | TokenTree::Punct(_) => {
                let leading =
                    is_colons(at) && matches!(tokens.get(at + 2), Some(TokenTree::Ident(_)));
                let mut cursor = if leading { at + 2 } else { at };
                let mut segments = Vec::new();
                while let Some(TokenTree::Ident(ident)) = tokens.get(cursor) {
                    segments.push(ident.to_string());
                    match is_colons(cursor + 1) {
                        true => cursor += 3,
                        false => break,
                    }
                }
                match segments.len() > 1 || (leading && !segments.is_empty()) {
                    true => {
                        paths.push((leading, segments));
                        at = cursor + 1;
                    }
                    false => at += 1,
                }
            }
            TokenTree::Literal(_) => at += 1,
        }
    }
}

/// The root a path names: its first segment, `::first`, or
/// `crate::<second>`.
fn path_root(leading: bool, segments: &[String]) -> Option<String> {
    match (leading, segments) {
        (true, [first, ..]) => Some(format!("::{first}")),
        (false, [first, second, ..]) if first == "crate" => Some(format!("crate::{second}")),
        (false, [first, _, ..]) => Some(first.clone()),
        _ => None,
    }
}

/// Every name `file` declares or imports at any depth: its own items,
/// generic parameters and `use` leaves.
#[derive(Default)]
struct Declared(BTreeSet<String>);
impl<'ast> Visit<'ast> for Declared {
    fn visit_item(&mut self, item: &'ast syn::Item) {
        let ident = match item {
            syn::Item::Struct(i) => Some(&i.ident),
            syn::Item::Enum(i) => Some(&i.ident),
            syn::Item::Fn(i) => Some(&i.sig.ident),
            syn::Item::Trait(i) => Some(&i.ident),
            syn::Item::Type(i) => Some(&i.ident),
            syn::Item::Const(i) => Some(&i.ident),
            syn::Item::Static(i) => Some(&i.ident),
            syn::Item::Mod(i) => Some(&i.ident),
            _ => None,
        };
        self.0.extend(ident.map(ToString::to_string));
        syn::visit::visit_item(self, item);
    }
    fn visit_generic_param(&mut self, param: &'ast syn::GenericParam) {
        if let syn::GenericParam::Type(t) = param {
            self.0.insert(t.ident.to_string());
        }
        syn::visit::visit_generic_param(self, param);
    }
    fn visit_use_name(&mut self, name: &'ast syn::UseName) {
        self.0.insert(name.ident.to_string());
    }
    fn visit_use_rename(&mut self, rename: &'ast syn::UseRename) {
        self.0.insert(rename.rename.to_string());
    }
}

/// Every full path a file names: in its code and types, inside its
/// macros, and (apart) every path its `use` trees import.
#[derive(Default)]
struct Named {
    paths: Vec<(bool, Vec<String>)>,
    uses: Vec<(bool, Vec<String>)>,
}
impl<'ast> Visit<'ast> for Named {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        let segments = path.segments.iter().map(|s| s.ident.to_string()).collect();
        self.paths.push((path.leading_colon.is_some(), segments));
        syn::visit::visit_path(self, path);
    }
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        token_paths(mac.tokens.clone(), &mut self.paths);
        syn::visit::visit_macro(self, mac);
    }
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        use_paths(
            &item.tree,
            item.leading_colon.is_some(),
            &[],
            &mut self.uses,
        );
    }
}

/// Every path `tree` imports under `prefix`.
fn use_paths(
    tree: &syn::UseTree,
    leading: bool,
    prefix: &[String],
    paths: &mut Vec<(bool, Vec<String>)>,
) {
    let with = |last: String| prefix.iter().cloned().chain([last]).collect::<Vec<_>>();
    match tree {
        syn::UseTree::Path(path) => {
            use_paths(&path.tree, leading, &with(path.ident.to_string()), paths)
        }
        syn::UseTree::Name(name) => paths.push((leading, with(name.ident.to_string()))),
        syn::UseTree::Rename(rename) => paths.push((leading, with(rename.ident.to_string()))),
        syn::UseTree::Glob(_) => paths.push((leading, with("*".to_owned()))),
        syn::UseTree::Group(group) => {
            for item in &group.items {
                use_paths(item, leading, prefix, paths);
            }
        }
    }
}

fn named(file: &syn::File) -> Named {
    let mut named = Named::default();
    named.visit_file(file);
    named
}

/// The roots `source` names that are neither its own nor allowed.
fn unapproved_roots(source: &str) -> Vec<String> {
    let file = syn::parse_file(source).expect("valid Rust");
    let mut declared = Declared::default();
    declared.visit_file(&file);
    let named = named(&file);
    let approved = |root: &String| {
        STD_ROOTS.contains(&root.as_str())
            || APPROVED_CRATES.contains(&root.as_str())
            || root == "crate::domain"
    };
    let paths = named
        .paths
        .iter()
        .filter_map(|(leading, segments)| path_root(*leading, segments))
        .filter(|root| {
            !(approved(root) || PRELUDE.contains(&root.as_str()) || declared.0.contains(root))
        });
    // A `use` root is never excused by the name it imports; a single
    // segment `use x;` names the crate `x`.
    let uses = named
        .uses
        .iter()
        .filter_map(|(leading, segments)| match (leading, segments.as_slice()) {
            (false, [only]) => Some(only.clone()),
            (leading, segments) => path_root(*leading, segments),
        })
        .filter(|root| !approved(root));
    let unapproved: BTreeSet<String> = paths.chain(uses).collect();
    unapproved.into_iter().collect()
}

#[test]
fn the_web_fetch_use_case_names_only_owned_std_url_and_domain_paths() {
    let source = fs::read_to_string(USE_CASE).expect("the use case");
    assert_eq!(unapproved_roots(&source), Vec::<String>::new());
}

#[test]
fn the_use_case_path_check_rejects_every_unlisted_vendor_spelling() {
    let clean = "use std::sync::Arc; use url::Url; use crate::domain::x::y;
        enum Gate { Allowed } fn f() -> Option<Gate> {
            let _ = vec![std::mem::size_of::<u8>()]; let _ = format!(\"{:?}\", Gate::Allowed);
            Some(Gate::Allowed) }";
    assert_eq!(unapproved_roots(clean), Vec::<String>::new());
    for (vendor, root) in [
        ("use reqwest::Client;", "reqwest"),
        ("use reqwest;", "reqwest"),
        ("fn f() { let _ = reqwest::Client::new(); }", "reqwest"),
        ("fn f() { let _ = ::tokio::spawn(async {}); }", "::tokio"),
        ("fn f() -> hyper::Uri { loop {} }", "hyper"),
        (
            "use crate::infrastructure::http::web_fetch;",
            "crate::infrastructure",
        ),
        ("fn f() { crate::interface::x(); }", "crate::interface"),
        ("use serde_json as json;", "serde_json"),
        ("use {std::sync::Arc, tokio::sync::Mutex};", "tokio"),
        (
            "use crate::{domain::x, application::y};",
            "crate::application",
        ),
        // #1942 final review: macro bodies, `super` and `self`.
        (
            "fn f() { let _ = vec![reqwest::Client::new()]; }",
            "reqwest",
        ),
        (
            "fn f() { let _ = format!(\"{:?}\", tokio::net::lookup_host(\"x\")); }",
            "tokio",
        ),
        (
            "fn f() { let _ = format!(\"{:?}\", ::tokio::spawn(g())); }",
            "::tokio",
        ),
        (
            "macro_rules! m { () => { reqwest::Client::new() } } fn f() { let _ = m!(); }",
            "reqwest",
        ),
        (
            "use super::http as h; fn f() { let _ = h::Client::new(); }",
            "super",
        ),
        ("fn f() { let _ = super::http::Client::new(); }", "super"),
        ("fn f() { let _ = self::inner::x(); } mod inner {}", "self"),
    ] {
        assert_eq!(unapproved_roots(vendor), vec![root.to_owned()], "{vendor}");
    }
    // A glob brings in names nothing declares: both are reported.
    assert_eq!(
        unapproved_roots("use super::*; fn f() { let _ = Client::new(); }"),
        vec!["Client".to_owned(), "super".to_owned()]
    );
    assert_eq!(
        unapproved_roots(
            "use self::inner::reqwest_alias as r; mod inner { pub use ::reqwest as reqwest_alias; } fn f(){ let _ = r::Client::new(); }"
        ),
        vec!["::reqwest".to_owned(), "self".to_owned()]
    );
}

/// The `reqwest` paths the adapter may name, exactly.
const ALLOWED_REQWEST_PATHS: &[&str] = &[
    "reqwest::Certificate",
    "reqwest::Client",
    "reqwest::Client::builder",
    "reqwest::ClientBuilder",
    "reqwest::ClientBuilder::add_root_certificate",
    "reqwest::Error",
    "reqwest::Response",
    "reqwest::dns::Addrs",
    "reqwest::dns::Name",
    "reqwest::dns::Resolve",
    "reqwest::dns::Resolving",
    "reqwest::redirect::Policy::none",
];
/// Any path under this prefix is allowed: header names, values and maps.
const ALLOWED_REQWEST_PREFIX: &str = "reqwest::header::";

/// The methods allowed on the chains rooted at each transport binding.
const ALLOWED_CHAIN_METHODS: &[(&str, &[&str])] = &[
    // The recipe's own settings on the fresh builder.
    ("builder", &["connect_timeout"]),
    // The one client build, enforcement last.
    (
        "recipe",
        &[
            "builder",
            "dns_resolver",
            "redirect",
            "no_proxy",
            "build",
            "map_err",
        ],
    ),
    // Each hop's request.
    ("client", &["get", "timeout", "header"]),
    ("request", &["header", "send", "map_err"]),
];

/// Each method call's name and the binding its receiver chain starts at.
#[derive(Default)]
struct Chains(Vec<(String, String)>);
impl Chains {
    fn root(expr: &syn::Expr) -> Option<String> {
        match expr {
            syn::Expr::MethodCall(call) => Self::root(&call.receiver),
            syn::Expr::Await(awaited) => Self::root(&awaited.base),
            syn::Expr::Try(tried) => Self::root(&tried.expr),
            syn::Expr::Paren(paren) => Self::root(&paren.expr),
            syn::Expr::Path(path) => path.path.get_ident().map(ToString::to_string),
            _ => None,
        }
    }
}
impl<'ast> Visit<'ast> for Chains {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        if let Some(root) = Self::root(&call.receiver) {
            self.0.push((root, call.method.to_string()));
        }
        syn::visit::visit_expr_method_call(self, call);
    }
}

/// Everything in the adapter `source` outside its allowlists.
fn adapter_violations(source: &str) -> Vec<String> {
    let file = syn::parse_file(source).expect("valid Rust");
    let named = named(&file);
    let reqwest_paths = named
        .paths
        .iter()
        .filter(|(_, segments)| segments.first().is_some_and(|s| s == "reqwest"))
        .map(|(_, segments)| segments.join("::"))
        .filter(|path| {
            !(ALLOWED_REQWEST_PATHS.contains(&path.as_str())
                || path.starts_with(ALLOWED_REQWEST_PREFIX))
        });
    // Every path stays spelled out in full: nothing of reqwest is imported.
    let reqwest_uses = named
        .uses
        .iter()
        .filter(|(_, segments)| segments.first().is_some_and(|s| s == "reqwest"))
        .map(|(_, segments)| format!("use {}", segments.join("::")));
    let mut chains = Chains::default();
    chains.visit_file(&file);
    let calls = chains.0.into_iter().filter_map(|(root, method)| {
        let allowed = ALLOWED_CHAIN_METHODS
            .iter()
            .find(|(binding, _)| *binding == root)?;
        (!allowed.1.contains(&method.as_str())).then(|| format!("{root}.{method}"))
    });
    let violations: BTreeSet<String> = reqwest_paths.chain(reqwest_uses).chain(calls).collect();
    violations.into_iter().collect()
}

#[test]
fn the_adapter_names_only_allowlisted_reqwest_paths_and_chain_methods() {
    let source = fs::read_to_string(ADAPTER).expect("the adapter");
    assert_eq!(adapter_violations(&source), Vec::<String>::new());
}

#[test]
fn the_adapter_allowlist_rejects_every_other_client_or_transport() {
    let clean = "fn f(recipe: R, client: reqwest::Client) {
        let builder = reqwest::Client::builder();
        let _ = builder.connect_timeout(t);
        let _ = recipe.builder().dns_resolver(r).redirect(reqwest::redirect::Policy::none()).no_proxy().build();
        let request = client.get(u).timeout(t).header(reqwest::header::USER_AGENT, v);
        let _ = request.send();
    }";
    assert_eq!(adapter_violations(clean), Vec::<String>::new());
    for (bypass, violation) in [
        (
            "fn f(u: url::Url) { let _ = reqwest::get(u); }",
            "reqwest::get",
        ),
        (
            "fn f() { let _ = reqwest::Client::new(); }",
            "reqwest::Client::new",
        ),
        (
            "fn f() { let _ = reqwest::Client::default(); }",
            "reqwest::Client::default",
        ),
        (
            "fn f() { let _ = vec![reqwest::Client::new()]; }",
            "reqwest::Client::new",
        ),
        ("fn f(p: reqwest::Proxy) {}", "reqwest::Proxy"),
        (
            "use reqwest::Client; fn f() { let _ = Client::new(); }",
            "use reqwest::Client",
        ),
        ("fn f(client: C) { let _ = client.post(u); }", "client.post"),
        (
            "fn f(recipe: R) { let _ = recipe.builder().proxy(p).no_proxy().build(); }",
            "recipe.proxy",
        ),
        (
            "fn f(recipe: R) { let _ = recipe.builder().unix_socket(p).no_proxy().build(); }",
            "recipe.unix_socket",
        ),
        (
            "fn f(recipe: R) { let _ = recipe.builder().resolve(n, a).no_proxy().build(); }",
            "recipe.resolve",
        ),
        (
            "fn f(builder: B) { let _ = builder.proxy(p); }",
            "builder.proxy",
        ),
        (
            "fn f(request: Q) { let _ = request.basic_auth(u, p); }",
            "request.basic_auth",
        ),
    ] {
        assert_eq!(
            adapter_violations(bypass),
            vec![violation.to_owned()],
            "{bypass}"
        );
    }
}

/// Every method call in the adapter: `(method, receiver's method)`, and
/// every public fn whose signature mentions `ClientBuilder`.
#[derive(Default)]
struct Calls(Vec<(String, Option<String>)>, Vec<String>);
impl<'ast> Visit<'ast> for Calls {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let receiver = match &*call.receiver {
            syn::Expr::MethodCall(inner) => Some(inner.method.to_string()),
            _ => None,
        };
        self.0.push((call.method.to_string(), receiver));
        syn::visit::visit_expr_method_call(self, call);
    }
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.visit_signature_shape(&item.vis, &item.sig);
        syn::visit::visit_item_fn(self, item);
    }
    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.visit_signature_shape(&item.vis, &item.sig);
        syn::visit::visit_impl_item_fn(self, item);
    }
}
impl Calls {
    fn visit_signature_shape(&mut self, vis: &syn::Visibility, sig: &syn::Signature) {
        let public = matches!(vis, syn::Visibility::Public(_));
        let text = quote::quote!(#sig).to_string();
        if public && text.contains("ClientBuilder") {
            self.1.push(sig.ident.to_string());
        }
    }
}

#[test]
fn the_web_fetch_client_is_built_once_from_the_recipe_with_no_proxy_last() {
    let source = fs::read_to_string(ADAPTER).expect("the adapter");
    let file = syn::parse_file(&source).expect("valid Rust");
    let mut calls = Calls::default();
    calls.visit_file(&file);
    let builds: Vec<_> = calls
        .0
        .iter()
        .filter(|(method, _)| method == "build")
        .collect();
    assert_eq!(
        builds,
        [&("build".to_owned(), Some("no_proxy".to_owned()))],
        "exactly one client build, directly after no_proxy"
    );
    let builders = named(&file)
        .paths
        .iter()
        .filter(|(_, segments)| segments.join("::") == "reqwest::Client::builder")
        .count();
    assert_eq!(builders, 1, "one reqwest::Client::builder(), in the recipe");
    assert_eq!(
        calls.1,
        Vec::<String>::new(),
        "no public fn takes or returns a ClientBuilder"
    );
    let recipe = file
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(s) if s.ident == "WebFetchClientRecipe" => Some(s),
            _ => None,
        })
        .expect("the recipe");
    let fields: Vec<String> = recipe
        .fields
        .iter()
        .filter_map(|field| field.ident.as_ref().map(ToString::to_string))
        .collect();
    assert_eq!(
        fields,
        ["connect_timeout", "root_certificates"],
        "the recipe holds only these settings: no transport can be configured"
    );
    let composition = fs::read_to_string("src/composition/web_fetch.rs").expect("composition");
    assert!(
        composition.contains("recipe: WebFetchClientRecipe")
            && !composition.contains("ClientBuilder"),
        "the composition takes the recipe, never a builder"
    );
    let cli = syn::parse_file(&fs::read_to_string("src/interface/cli/mod.rs").expect("cli"))
        .expect("valid Rust");
    let factory = cli
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Type(t) if t.ident == "WebFetchToolFactory" => Some(&t.ty),
            _ => None,
        })
        .expect("the factory type");
    let factory = quote::quote!(#factory).to_string();
    assert!(
        factory.contains("WebFetchClientRecipe") && !factory.contains("ClientBuilder"),
        "{factory}"
    );
}
