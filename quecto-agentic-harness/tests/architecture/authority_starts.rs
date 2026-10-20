//! Who may start an authority server (#1910, #2622 review).
//!
//! The start functions are `pub(crate)`, so the compiler refuses a start
//! from outside the crate. Inside it, this parse-based scan resolves every
//! path through every alias the crate and the file declare (`use … as S`,
//! `pub use … as Broker`, `type S = …`; a name aliased twice keeps both
//! targets) and qualified-self paths (`<AuthorityServer>::start`), and reads
//! the tokens of every macro invocation and `macro_rules!` body, so only the
//! server itself, composition and the broker's run path name a server item
//! other than its known non-starting ones. A plain substring needle backs
//! the parse up, fail-closed. The test-support start functions in
//! composition stay behind their gate.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use proc_macro2::{TokenStream, TokenTree};

use super::teardown_authority::walk;
use super::use_case_construction::{crate_raw_aliases, ident_name};

const SERVER: &str = "AuthorityServer";

/// The server's items that start nothing: anything else named through the
/// server outside `STARTERS` counts as a start (an allowlist, so a new start
/// variant is caught without being listed).
const NON_STARTING: &[&str] = &["directory", "shutdown"];

/// The only files that may name the server at all (#2627 review): its module
/// and re-export, composition, the broker's run path, and the tests that
/// hold or inspect a running one. A new file naming it must be listed, so a
/// spelling the parse cannot follow still has to pass review.
const SERVER_NAMERS: &[&str] = &[
    "src/composition/admission.rs",
    "src/infrastructure/admission/mod.rs",
    "src/infrastructure/admission/server.rs",
    "src/interface/cli/admission_broker.rs",
    "tests/bdd/inference_admission_authority_steps.rs",
    "tests/contracts/admission_observation.rs",
    "tests/contracts/authority_admin.rs",
    "tests/integration/inference_admission_broker.rs",
];

/// The scan's own fixtures spell starts in string literals; the parse never
/// reads a literal as a path, but the substring backstop would.
const BACKSTOP_EXEMPT: &[&str] = &["tests/architecture/authority_starts.rs"];

/// The files that may start an authority server: its own module, composition
/// and the `admission-broker run` path.
const STARTERS: &[&str] = &[
    "src/infrastructure/admission/server.rs",
    "src/composition/admission.rs",
    "src/interface/cli/admission_broker.rs",
];

struct Starts<'a> {
    aliases: &'a BTreeMap<String, BTreeSet<String>>,
    impl_types: Vec<String>,
    found: Vec<String>,
}

impl Starts<'_> {
    /// Whether `ident` (or `Self` in the enclosing impl) names the server,
    /// through any chain of aliases.
    fn names_server(&self, ident: &str) -> bool {
        let start = match ident {
            "Self" => match self.impl_types.last() {
                Some(ty) => ty.clone(),
                None => return false,
            },
            other => other.to_string(),
        };
        // Every chain of aliases from the name, not just one: a name two
        // aliases share is the server if either reaches it.
        let mut seen = BTreeSet::new();
        let mut pending = vec![start];
        while let Some(name) = pending.pop() {
            if name == SERVER {
                return true;
            }
            if seen.insert(name.clone()) {
                pending.extend(self.aliases.get(&name).into_iter().flatten().cloned());
            }
        }
        false
    }

    fn type_names_server(&self, ty: &syn::Type) -> bool {
        match ty {
            syn::Type::Paren(ty) => self.type_names_server(&ty.elem),
            // A type a macro produces: any token of it naming the server.
            syn::Type::Macro(ty) => {
                let mut tokens = Vec::new();
                flatten(ty.mac.tokens.clone(), &mut tokens);
                tokens.iter().any(|token| {
                    matches!(token, TokenTree::Ident(ident) if self.names_server(&ident_name(ident)))
                })
            }
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
            && !NON_STARTING.contains(&last.as_str())
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

    /// A macro's arguments (and a `macro_rules!` body) are tokens the syntax
    /// visitor does not walk, and a macro can assemble a path from pieces, so
    /// inside one every token that names the server counts as a start unless
    /// it is followed by `:: directory` or `:: shutdown` (#2627 review).
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        let mut tokens = Vec::new();
        flatten(mac.tokens.clone(), &mut tokens);
        for (at, token) in tokens.iter().enumerate() {
            let TokenTree::Ident(owner) = token else {
                continue;
            };
            if !self.names_server(&ident_name(owner)) {
                continue;
            }
            let non_starting = matches!(
                tokens.get(at + 1..at + 4),
                Some([TokenTree::Punct(first), TokenTree::Punct(second), TokenTree::Ident(item)])
                    if first.as_char() == ':'
                        && second.as_char() == ':'
                        && NON_STARTING.contains(&ident_name(item).as_str())
            );
            if !non_starting {
                self.found.push(format!("{owner} (in a macro)"));
            }
        }
        syn::visit::visit_macro(self, mac);
    }
}

/// Every token of `stream`, groups opened, in order.
fn flatten(stream: TokenStream, out: &mut Vec<TokenTree>) {
    for tree in stream {
        match tree {
            TokenTree::Group(group) => flatten(group.stream(), out),
            other => out.push(other),
        }
    }
}

/// Every rename (`… as Y`) and `type Y = …::X` of a parsed file, each pair
/// kept: a name renamed twice keeps both targets.
fn alias_pairs(file: &syn::File) -> Vec<(String, String)> {
    use syn::visit::Visit;
    struct Pairs(Vec<(String, String)>);
    impl<'ast> Visit<'ast> for Pairs {
        fn visit_use_rename(&mut self, item: &'ast syn::UseRename) {
            self.0
                .push((ident_name(&item.rename), ident_name(&item.ident)));
        }
        fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
            if let syn::Type::Path(ty) = &*item.ty
                && let Some(last) = ty.path.segments.last()
            {
                self.0
                    .push((ident_name(&item.ident), ident_name(&last.ident)));
            }
            syn::visit::visit_item_type(self, item);
        }
    }
    let mut pairs = Pairs(Vec::new());
    pairs.visit_file(file);
    pairs.0
}

/// Every alias the crate declares, with every target a name has.
fn crate_aliases() -> BTreeMap<String, BTreeSet<String>> {
    let mut aliases: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (alias, target) in crate_raw_aliases() {
        aliases
            .entry(alias.clone())
            .or_default()
            .insert(target.clone());
    }
    let mut files = Vec::new();
    walk(Path::new("src"), &mut files);
    walk(Path::new("tests"), &mut files);
    for file in files {
        if let Ok(parsed) = syn::parse_file(&std::fs::read_to_string(&file).unwrap()) {
            for (alias, target) in alias_pairs(&parsed) {
                aliases.entry(alias).or_default().insert(target);
            }
        }
    }
    aliases
}

fn crate_aliases_cached() -> &'static BTreeMap<String, BTreeSet<String>> {
    use std::sync::OnceLock;
    static ALIASES: OnceLock<BTreeMap<String, BTreeSet<String>>> = OnceLock::new();
    ALIASES.get_or_init(crate_aliases)
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
    let mut aliases = crate_aliases_cached().clone();
    for (alias, target) in alias_pairs(&file) {
        aliases.entry(alias).or_default().insert(target);
    }
    let mut scan = Starts {
        aliases: &aliases,
        impl_types: Vec::new(),
        found: Vec::new(),
    };
    scan.visit_file(&file);
    scan.found
}

/// What a file outside `STARTERS` is refused for: every start the parse
/// finds, and, fail-closed, the spelling `AuthorityServer::start` anywhere in
/// its text (a comment, a string, tokens the parse could not read).
fn outside_starts(file: &str, source: &str) -> Vec<String> {
    let mut found: Vec<String> = starts_in(source)
        .into_iter()
        .map(|start| format!("{file}: {start}"))
        .collect();
    if !BACKSTOP_EXEMPT.contains(&file) && source.contains(concat!("AuthorityServer", "::start")) {
        found.push(format!("{file}: names `AuthorityServer::start…`"));
    }
    found
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
        let source = std::fs::read_to_string(&file).unwrap();
        if STARTERS.contains(&file.as_str()) {
            allowed += starts_in(&source).len();
        } else {
            outside.extend(outside_starts(&file, &source));
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
        // Inside a macro's arguments (#2622 review round 3).
        "fn f() { assert!(std::mem::size_of_val(&crate::infrastructure::admission::AuthorityServer::start) == 0); }",
        "async fn f() { assert!(AuthorityServer::start(d, p).await.is_ok()); }",
        "async fn f() { assert!(matches!(AuthorityServer::start(d, p).await, Err(_))); }",
        "async fn f() { tokio::select! { _ = AuthorityServer::start(d, p) => {} } }",
        "macro_rules! go { () => { AuthorityServer::start(d, p) }; }",
        // A start variant nobody listed is still a start.
        "fn f() { AuthorityServer::begin(d, p); }",
        // A name aliased twice: either target may be the server.
        "use crate::infrastructure::admission::AuthorityServer as S;\nuse crate::other::Thing as S;\nfn f() { S::start(d, p); }",
        // Review (#2627 round 3): pieces a macro assembles.
        "async fn f() { assert!(<AuthorityServer>::start(d, p).await.is_ok()); }",
        "async fn f() { assert!(AuthorityServer::<>::start(d, p).await.is_ok()); }",
        "macro_rules! go { ($s:ident) => { $s::start(d, p) }; }\nfn f() { go!(AuthorityServer); }",
        "fn f() { let _ = <srv!()>::start(d, p); }\nmacro_rules! srv { () => { AuthorityServer }; }",
        "type Zz = AuthorityServer;\nasync fn f() { assert!(<Zz>::start(d, p).await.is_ok()); }",
        "use crate::other::Thing as S;\nuse crate::infrastructure::admission::AuthorityServer as S;\nfn f() { S::start(d, p); }",
    ];
    for source in caught {
        assert!(!starts_in(source).is_empty(), "must catch:\n{source}");
    }
    let spared = [
        "fn f() { Other::start(d, p); }",
        "fn f(s: &AuthorityServer) { s.directory(); }",
        "fn f() { AuthorityServer::directory(s); }",
        "fn f() { AuthorityServer::shutdown(s); }",
        "fn f() { assert!(Other::start(d, p).is_ok()); }",
        "fn f() { assert_eq!(AuthorityServer::directory(s), d); }",
        "fn f() { assert!(AuthorityServer::shutdown(s).is_ok()); }",
        "impl Other { fn again() { Self::start(d, p); } }",
        "fn f() { start_authority(d, p); }",
    ];
    for source in spared {
        let found = starts_in(source);
        assert!(found.is_empty(), "must spare:\n{source}\nfound {found:?}");
    }
}

/// The substring backstop refuses the spelling the parse cannot see, and
/// spares only the scan's own fixtures.
#[test]
fn the_start_backstop_is_fail_closed() {
    let hidden = concat!("// started with AuthorityServer", "::start elsewhere\n");
    assert_eq!(
        starts_in(hidden),
        Vec::<String>::new(),
        "a comment is no path"
    );
    assert_eq!(outside_starts("src/elsewhere.rs", hidden).len(), 1);
    assert_eq!(
        outside_starts("tests/architecture/authority_starts.rs", hidden),
        Vec::<String>::new()
    );
}

/// Exactly `SERVER_NAMERS` (and this scan) name the server anywhere in `src`
/// or `tests`, in code, comments or strings.
#[test]
fn only_the_listed_files_name_the_authority_server() {
    let mut files = Vec::new();
    walk(Path::new("src"), &mut files);
    walk(Path::new("tests"), &mut files);
    let namers: BTreeSet<String> = files
        .into_iter()
        .filter(|file| !BACKSTOP_EXEMPT.contains(&file.as_str()))
        .filter(|file| std::fs::read_to_string(file).unwrap().contains(SERVER))
        .collect();
    let listed: BTreeSet<String> = SERVER_NAMERS.iter().map(|file| file.to_string()).collect();
    assert_eq!(
        namers, listed,
        "the files naming AuthorityServer are SERVER_NAMERS"
    );
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
