//! #1942 ledger rows 7, 11, 13 and 14: bounded structural proofs for
//! web_fetch, paired with the adapter's execute-path tests (these prove
//! shapes, not runtime behaviour).
//!
//! - The application use case names only its own items, the standard
//!   library, the approved `url` crate and the crate's pure domain, in its
//!   imports and in every path it writes.
//! - The adapter's client is built in exactly one place, from the owned
//!   recipe, with `no_proxy` as the very last setting before `build`, and
//!   no function takes or returns a `reqwest::ClientBuilder` publicly.
use std::collections::BTreeSet;
use std::fs;
use syn::visit::Visit;

const USE_CASE: &str = "src/application/agent_turn/use_cases/web_fetch.rs";
const ADAPTER: &str = "src/infrastructure/http/web_fetch.rs";

/// Roots every file may name: the language and standard library.
const STD_ROOTS: &[&str] = &["std", "core", "alloc", "self", "super", "Self"];
/// The standard prelude's multi-segment roots.
const PRELUDE: &[&str] = &[
    "Option", "Some", "None", "Result", "Ok", "Err", "String", "Vec", "Box", "u8", "u16", "u32",
    "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32", "f64", "char",
    "str", "bool",
];
/// The one external crate the use case may name.
const APPROVED_CRATES: &[&str] = &["url"];

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

/// The roots of every multi-segment path in `file` and, apart, of every
/// `use` tree (a `use` root is never excused by the name it imports), with
/// `crate::` paths kept to their second segment (`crate::domain`).
#[derive(Default)]
struct Named(BTreeSet<String>, BTreeSet<String>);
impl Named {
    fn root(path: &syn::Path) -> Option<String> {
        let mut segments = path.segments.iter().map(|s| s.ident.to_string());
        let first = segments.next()?;
        let second = segments.next();
        match (path.leading_colon.is_some(), first.as_str(), second) {
            (true, _, _) => Some(format!("::{first}")),
            (false, "crate", Some(second)) => Some(format!("crate::{second}")),
            (false, _, Some(_)) => Some(first),
            (false, _, None) => None,
        }
    }
}
impl<'ast> Visit<'ast> for Named {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.0.extend(Self::root(path));
        syn::visit::visit_path(self, path);
    }
    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        use_roots(&item.tree, &[], &mut self.1);
    }
}

/// The root of every import in `tree` under `prefix`: its first segment, or
/// `crate::<second>`.
fn use_roots(tree: &syn::UseTree, prefix: &[String], roots: &mut BTreeSet<String>) {
    let root = |last: String| {
        let path: Vec<String> = prefix.iter().cloned().chain([last]).collect();
        match path.as_slice() {
            [first, second, ..] if first == "crate" => format!("crate::{second}"),
            [first, ..] => first.clone(),
            [] => unreachable!("a path has a segment"),
        }
    };
    match tree {
        syn::UseTree::Path(path) if prefix.len() < 2 => {
            let deeper: Vec<String> = prefix
                .iter()
                .cloned()
                .chain([path.ident.to_string()])
                .collect();
            use_roots(&path.tree, &deeper, roots);
        }
        syn::UseTree::Path(path) => {
            roots.insert(root(path.ident.to_string()));
        }
        syn::UseTree::Name(name) => {
            roots.insert(root(name.ident.to_string()));
        }
        syn::UseTree::Rename(rename) => {
            roots.insert(root(rename.ident.to_string()));
        }
        syn::UseTree::Glob(_) => {
            roots.insert(root("*".to_owned()));
        }
        syn::UseTree::Group(group) => {
            for item in &group.items {
                use_roots(item, prefix, roots);
            }
        }
    }
}

/// The roots `source` names that are neither its own nor allowed.
fn unapproved_roots(source: &str) -> Vec<String> {
    let file = syn::parse_file(source).expect("valid Rust");
    let mut declared = Declared::default();
    declared.visit_file(&file);
    let mut named = Named::default();
    named.visit_file(&file);
    let approved = |root: &String| {
        STD_ROOTS.contains(&root.as_str())
            || APPROVED_CRATES.contains(&root.as_str())
            || root == "crate::domain"
    };
    let paths = named.0.into_iter().filter(|root| {
        !(approved(root) || PRELUDE.contains(&root.as_str()) || declared.0.contains(root))
    });
    let uses = named.1.into_iter().filter(|root| !approved(root));
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
        enum Gate { Allowed } fn f() -> Option<Gate> { let _ = std::mem::size_of::<u8>(); Some(Gate::Allowed) }";
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
    ] {
        assert_eq!(unapproved_roots(vendor), vec![root.to_owned()], "{vendor}");
    }
}

/// Every method-call chain in the adapter: `(method, receiver's method)`.
#[derive(Default)]
struct Calls(Vec<(String, Option<String>)>, usize, Vec<String>);
impl<'ast> Visit<'ast> for Calls {
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let receiver = match &*call.receiver {
            syn::Expr::MethodCall(inner) => Some(inner.method.to_string()),
            _ => None,
        };
        self.0.push((call.method.to_string(), receiver));
        syn::visit::visit_expr_method_call(self, call);
    }
    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        let text: Vec<String> = path
            .path
            .segments
            .iter()
            .map(|s| s.ident.to_string())
            .collect();
        if text.ends_with(&["Client".to_owned(), "builder".to_owned()]) {
            self.1 += 1;
        }
        syn::visit::visit_expr_path(self, path);
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
    /// Records a public function whose signature mentions `ClientBuilder`.
    fn visit_signature_shape(&mut self, vis: &syn::Visibility, sig: &syn::Signature) {
        let public = matches!(vis, syn::Visibility::Public(_));
        let text = quote::quote!(#sig).to_string();
        if public && text.contains("ClientBuilder") {
            self.2.push(sig.ident.to_string());
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
    assert_eq!(calls.1, 1, "one reqwest::Client::builder(), in the recipe");
    assert_eq!(
        calls.2,
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
