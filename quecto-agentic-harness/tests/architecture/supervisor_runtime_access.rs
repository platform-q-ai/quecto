//! Who may reach the supervisor's runtime (#2286 review round 4), proven
//! over every function of every checked file — free functions, any impl
//! (whatever its self type), trait default bodies, nested functions and
//! module scope alike, and the tokens of every macro invocation:
//!
//! - the checked files are those that can see the supervisor's private
//!   fields (its own file and every file of its module tree) and those
//!   whose code runs on its runtime (the line pumps, the stderr tail).
//!   A module declared in one of them lives in another of them: no
//!   `#[path]` but a `#[cfg(test)]` `*_tests.rs` sibling;
//! - a runtime field of the supervisor (`.runtime`, `.handle`: each field
//!   whose type names a tokio runtime) is read or destructured, and a
//!   runtime is spawned on, blocked on or entered (`.spawn(..)` and kin),
//!   only by the allowlisted helpers, each at its pinned visibility;
//! - code already running on the runtime cannot reach it without a field:
//!   no path names `spawn`, `spawn_blocking`, `spawn_local`,
//!   `block_in_place`, `block_on`, `current` or `try_current`
//!   (`tokio::spawn`, `Handle::current`, a `use … as` of them);
//! - no shape can carry a caller's code: no `dyn` trait object, type
//!   parameter, `impl Trait` parameter or fn pointer but the pinned
//!   termination protocol's;
//! - only allowlisted macros are invoked (no `macro_rules!`, no
//!   `include!`), and their tokens are held to the same rules.
//!
//! Every allowlist entry must still be needed.

use std::collections::BTreeSet;

use proc_macro2::{Spacing, TokenStream, TokenTree};
use quote::ToTokens;
use syn::visit::Visit;

use super::{PIPES, SUPERVISOR, TASKS};

const TAIL: &str = "src/infrastructure/processes/child_stderr_tail.rs";

/// The supervisor's module tree: every file in it sees its private fields.
const SUPERVISOR_TREE: &str = "src/infrastructure/processes/owned_child_supervisor/";

/// The files outside the supervisor's module tree whose code runs on its
/// runtime.
const RUNTIME_ONLY_FILES: &[&str] = &[PIPES, TAIL];

/// The supervisor's fields that give access to its runtime: exactly those
/// whose type names a tokio runtime.
pub(super) const RUNTIME_FIELDS: &[&str] = &["runtime", "handle"];

/// Methods that run code on a runtime, block on one or enter one.
const RUNTIME_METHODS: &[&str] = &[
    "spawn",
    "spawn_blocking",
    "spawn_local",
    "spawn_on",
    "block_on",
    "enter",
];

/// Path segments by which code already on a runtime reaches it.
const RUNTIME_PATH_WORDS: &[&str] = &[
    "spawn",
    "spawn_blocking",
    "spawn_local",
    "block_in_place",
    "block_on",
    "current",
    "try_current",
];

/// The sites that may reach the supervisor's runtime: file, site, the
/// visibility the site's function must keep, and why.
const RUNTIME_REACHERS: &[(&str, &str, &str, &str)] = &[
    (
        SUPERVISOR,
        "impl Drop for OwnedChildSupervisor::drop",
        "",
        "shuts the runtime down in the background, never blocking",
    ),
    (
        SUPERVISOR,
        "impl OwnedChildSupervisor::spawn",
        "pub",
        "spawns a Command on the runtime and adopts it",
    ),
    (
        SUPERVISOR,
        "impl OwnedChildSupervisor::retain_stderr_tail",
        "pub",
        "the stderr tail's drain: a pipe type in, bytes out",
    ),
    (
        SUPERVISOR,
        "impl OwnedChildSupervisor::retain_stderr_tail_within",
        "pub",
        "the stderr tail's drain: a pipe type in, bytes out",
    ),
    (
        SUPERVISOR,
        "impl OwnedChildSupervisor::adopt",
        "",
        "the reap task of an adopted child",
    ),
    (
        SUPERVISOR,
        "impl OwnedChildSupervisor::retire_when_reaped",
        "pub",
        "waits for the reap, then retires",
    ),
    (
        SUPERVISOR,
        "impl OwnedChildSupervisor::spawn_termination",
        "",
        "the private spawn helper of every requested termination",
    ),
    (
        SUPERVISOR,
        "impl OwnedChildSupervisor::spawn_pipe_task",
        "",
        "the private spawn helper of the line pumps: a PipeTask, built only by child_line_pipes",
    ),
    (
        TAIL,
        "impl StderrTail::pump_within",
        "pub",
        "drains a pipe on the handle the supervisor passes it",
    ),
];

/// The code-carrying shapes allowed: file, site and the shape. Only the
/// termination protocol, which is the supervisor's own (#1935).
const CODE_CARRIERS: &[(&str, &str, &str)] = &[
    (
        SUPERVISOR,
        "impl OwnedChildSupervisor::terminate",
        "type parameter P",
    ),
    (
        SUPERVISOR,
        "impl OwnedChildSupervisor::request_termination",
        PROTOCOL_OBJECT,
    ),
    (
        SUPERVISOR,
        "impl OwnedChildSupervisor::spawn_termination",
        PROTOCOL_OBJECT,
    ),
    (
        TASKS,
        "impl OwnedChildSupervisor::request_termination_observed",
        PROTOCOL_OBJECT,
    ),
];

const PROTOCOL_OBJECT: &str = "dyn Future < Output = ProtocolOutcome > + Send";

/// The macros the checked files may invoke, by path.
const MACROS: &[&str] = &[
    "assert",
    "debug_assert",
    "format",
    "matches",
    "tokio :: select",
    "tracing :: info",
    "tracing :: warn",
    "write",
];

/// What one site does.
enum Kind {
    /// Reaches the runtime: a field, a runtime method, a runtime path.
    Reach,
    /// Declares a shape that can carry code.
    Carrier,
    /// Invokes a macro.
    Macro(String),
    /// Declares a module outside the checked files.
    Module,
}

struct Finding {
    file: String,
    site: String,
    /// The visibility of the site's innermost function.
    visibility: String,
    kind: Kind,
    what: String,
}

/// The checked files, from `files`: the supervisor's module tree and the
/// runtime-only files. Each is asserted present.
pub(super) fn checked(files: &[(String, syn::File)]) -> Vec<&(String, syn::File)> {
    let checked: Vec<_> = files
        .iter()
        .filter(|(path, _)| {
            path == SUPERVISOR
                || path.starts_with(SUPERVISOR_TREE)
                || RUNTIME_ONLY_FILES.contains(&path.as_str())
        })
        .collect();
    for required in [SUPERVISOR, TASKS].iter().chain(RUNTIME_ONLY_FILES) {
        assert!(
            checked.iter().any(|(path, _)| path == required),
            "{required} is a checked file"
        );
    }
    checked
}

/// Every way `files` reach the supervisor's runtime or carry code onto it
/// outside the allowlists, and every stale allowlist entry.
pub(super) fn violations(files: &[(String, syn::File)]) -> Vec<String> {
    let checked = checked(files);
    let names: BTreeSet<&str> = checked.iter().map(|(path, _)| path.as_str()).collect();
    let mut findings = Vec::new();
    for (path, parsed) in &checked {
        let mut sites = Sites {
            file: path,
            checked: &names,
            scope: Vec::new(),
            visibility: Vec::new(),
            in_inputs: false,
            found: Vec::new(),
        };
        sites.visit_file(parsed);
        findings.extend(sites.found);
    }
    let mut out = Vec::new();
    let mut reachers = BTreeSet::new();
    let mut carriers = BTreeSet::new();
    let mut macros = BTreeSet::new();
    for finding in &findings {
        let at = format!("{}: `{}`", finding.file, finding.site);
        match &finding.kind {
            Kind::Reach => {
                let allowed = RUNTIME_REACHERS
                    .iter()
                    .find(|(file, site, _, _)| *file == finding.file && *site == finding.site);
                let Some((_, site, visibility, _)) = allowed else {
                    out.push(format!(
                        "{at} reaches the supervisor's runtime ({}): only the \
                         RUNTIME_REACHERS may",
                        finding.what
                    ));
                    continue;
                };
                if finding.visibility != *visibility {
                    out.push(format!(
                        "{at} is `{}`, not its pinned `{visibility}`",
                        finding.visibility
                    ));
                }
                reachers.insert(*site);
            }
            Kind::Carrier => {
                let allowed = CODE_CARRIERS.iter().find(|(file, site, what)| {
                    *file == finding.file && *site == finding.site && *what == finding.what
                });
                match allowed {
                    Some(entry) => {
                        carriers.insert(entry);
                    }
                    None => out.push(format!(
                        "{at} declares {}, which can carry a caller's code onto \
                         the supervisor's runtime",
                        finding.what
                    )),
                }
            }
            Kind::Macro(path) => match MACROS.iter().find(|name| **name == path) {
                Some(name) => {
                    macros.insert(*name);
                }
                None => out.push(format!(
                    "{at} invokes {}, which is not one of the checked MACROS",
                    finding.what
                )),
            },
            Kind::Module => out.push(format!("{at} {}", finding.what)),
        }
    }
    for (_, site, _, _) in RUNTIME_REACHERS {
        if !reachers.contains(site) {
            out.push(format!(
                "RUNTIME_REACHERS lists `{site}`, which reaches no runtime: remove it"
            ));
        }
    }
    for entry in CODE_CARRIERS {
        if !carriers.contains(entry) {
            out.push(format!(
                "CODE_CARRIERS lists `{}` at `{}`, which is not there: remove it",
                entry.2, entry.1
            ));
        }
    }
    for name in MACROS {
        if !macros.contains(name) {
            out.push(format!(
                "MACROS lists `{name}`, which no checked file invokes: remove it"
            ));
        }
    }
    out
}

fn tokens(item: &impl ToTokens) -> String {
    item.to_token_stream().to_string()
}

fn has_attribute(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident(name))
}

fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs
        .iter()
        .any(|attr| attr.path().is_ident("cfg") && tokens(&attr.meta) == "cfg (test)")
}

/// A `#[path = "<name>_tests.rs"]` in the declaring file's own directory.
fn is_sibling_test_path(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| match &attr.meta {
        syn::Meta::NameValue(pair) if pair.path.is_ident("path") => matches!(
            &pair.value,
            syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(path), .. })
                if path.value().ends_with("_tests.rs") && !path.value().contains('/')
        ),
        _ => false,
    })
}

struct Sites<'a> {
    file: &'a str,
    checked: &'a BTreeSet<&'a str>,
    scope: Vec<String>,
    visibility: Vec<String>,
    /// Inside a function's parameters, where `impl Trait` takes code.
    in_inputs: bool,
    found: Vec<Finding>,
}

impl Sites<'_> {
    fn record(&mut self, kind: Kind, what: String) {
        let site = if self.scope.is_empty() {
            "<module scope>".to_string()
        } else {
            self.scope.join("::")
        };
        self.found.push(Finding {
            file: self.file.to_string(),
            site,
            visibility: self.visibility.last().cloned().unwrap_or_default(),
            kind,
            what,
        });
    }

    fn scoped(&mut self, name: String, visit: impl FnOnce(&mut Self)) {
        self.scope.push(name);
        visit(self);
        self.scope.pop();
    }

    fn function(
        &mut self,
        name: String,
        visibility: &syn::Visibility,
        visit: impl FnOnce(&mut Self),
    ) {
        self.visibility.push(tokens(visibility));
        self.scoped(name, visit);
        self.visibility.pop();
    }

    fn path_words(&mut self, idents: impl IntoIterator<Item = String>, what: String) {
        if idents
            .into_iter()
            .any(|ident| RUNTIME_PATH_WORDS.contains(&ident.as_str()))
        {
            self.record(Kind::Reach, what);
        }
    }

    /// Hold a macro's tokens to the same rules: a runtime field or method
    /// after a `.`, a runtime word, a code-carrying keyword, and each macro
    /// invoked inside it.
    fn macro_tokens(&mut self, stream: TokenStream) {
        let mut previous: Option<TokenTree> = None;
        for tree in stream {
            let after_dot = matches!(&previous, Some(TokenTree::Punct(p)) if p.as_char() == '.');
            match &tree {
                TokenTree::Group(group) => self.macro_tokens(group.stream()),
                TokenTree::Punct(punct)
                    if punct.as_char() == '!' && punct.spacing() == Spacing::Alone =>
                {
                    if let Some(TokenTree::Ident(name)) = &previous {
                        self.record(
                            Kind::Macro(name.to_string()),
                            format!("`{name}!` in a macro"),
                        );
                    }
                }
                TokenTree::Ident(ident) => {
                    let word = ident.to_string();
                    let word = word.as_str();
                    if after_dot
                        && (RUNTIME_FIELDS.contains(&word) || RUNTIME_METHODS.contains(&word))
                    {
                        self.record(Kind::Reach, format!("`.{word}` in a macro"));
                    } else if RUNTIME_PATH_WORDS.contains(&word) {
                        self.record(Kind::Reach, format!("`{word}` in a macro"));
                    } else if CARRIER_KEYWORDS.contains(&word) {
                        self.record(Kind::Carrier, format!("`{word}` in a macro"));
                    }
                }
                TokenTree::Punct(_) | TokenTree::Literal(_) => {}
            }
            previous = Some(tree);
        }
    }
}

/// Keywords that, inside a macro's tokens, can declare a shape that
/// carries code.
const CARRIER_KEYWORDS: &[&str] = &["dyn", "impl", "fn"];

impl<'ast> Visit<'ast> for Sites<'_> {
    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        let name = match &item.trait_ {
            Some((_, path, _)) => format!("impl {} for {}", tokens(path), tokens(&item.self_ty)),
            None => format!("impl {}", tokens(&item.self_ty)),
        };
        self.scoped(name, |this| syn::visit::visit_item_impl(this, item));
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.function(item.sig.ident.to_string(), &item.vis, |this| {
            syn::visit::visit_impl_item_fn(this, item)
        });
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.function(item.sig.ident.to_string(), &item.vis, |this| {
            syn::visit::visit_item_fn(this, item)
        });
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        self.function(
            item.sig.ident.to_string(),
            &syn::Visibility::Inherited,
            |this| syn::visit::visit_trait_item_fn(this, item),
        );
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        self.scoped(format!("trait {}", item.ident), |this| {
            syn::visit::visit_item_trait(this, item)
        });
    }

    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        self.scoped(format!("struct {}", item.ident), |this| {
            syn::visit::visit_item_struct(this, item)
        });
    }

    fn visit_item_enum(&mut self, item: &'ast syn::ItemEnum) {
        self.scoped(format!("enum {}", item.ident), |this| {
            syn::visit::visit_item_enum(this, item)
        });
    }

    fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
        self.scoped(format!("type {}", item.ident), |this| {
            syn::visit::visit_item_type(this, item)
        });
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        let name = item.ident.to_string();
        if item.content.is_none() {
            // Allowed: a `#[cfg(test)]` `*_tests.rs` sibling, or a module
            // at its default path that is itself a checked file.
            let resolved = format!("{}/{name}.rs", self.file.trim_end_matches(".rs"));
            let allowed = if has_attribute(&item.attrs, "path") {
                is_cfg_test(&item.attrs) && is_sibling_test_path(&item.attrs)
            } else {
                self.checked.contains(resolved.as_str())
            };
            if !allowed {
                self.record(
                    Kind::Module,
                    format!(
                        "declares `mod {name}`, which is neither a `#[cfg(test)]` \
                         `*_tests.rs` sibling nor a checked file ({resolved})"
                    ),
                );
            }
        }
        self.scoped(format!("mod {name}"), |this| {
            syn::visit::visit_item_mod(this, item)
        });
    }

    fn visit_signature(&mut self, signature: &'ast syn::Signature) {
        self.visit_generics(&signature.generics);
        self.in_inputs = true;
        for input in &signature.inputs {
            self.visit_fn_arg(input);
        }
        self.in_inputs = false;
        self.visit_return_type(&signature.output);
    }

    fn visit_expr_field(&mut self, field: &'ast syn::ExprField) {
        if let syn::Member::Named(name) = &field.member
            && RUNTIME_FIELDS.contains(&name.to_string().as_str())
        {
            self.record(Kind::Reach, format!("`.{name}`"));
        }
        syn::visit::visit_expr_field(self, field);
    }

    fn visit_field_pat(&mut self, field: &'ast syn::FieldPat) {
        if let syn::Member::Named(name) = &field.member
            && RUNTIME_FIELDS.contains(&name.to_string().as_str())
        {
            self.record(Kind::Reach, format!("destructures `{name}`"));
        }
        syn::visit::visit_field_pat(self, field);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let method = call.method.to_string();
        if RUNTIME_METHODS.contains(&method.as_str()) {
            self.record(Kind::Reach, format!("`.{method}`"));
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.path_words(
            path.segments
                .iter()
                .map(|segment| segment.ident.to_string()),
            format!("`{}`", tokens(path)),
        );
        syn::visit::visit_path(self, path);
    }

    fn visit_use_name(&mut self, name: &'ast syn::UseName) {
        self.path_words([name.ident.to_string()], format!("`use … {}`", name.ident));
    }

    fn visit_use_rename(&mut self, rename: &'ast syn::UseRename) {
        self.path_words(
            [rename.ident.to_string(), rename.rename.to_string()],
            format!("`use … {} as {}`", rename.ident, rename.rename),
        );
    }

    fn visit_use_path(&mut self, path: &'ast syn::UsePath) {
        self.path_words(
            [path.ident.to_string()],
            format!("`use {} :: …`", path.ident),
        );
        syn::visit::visit_use_path(self, path);
    }

    fn visit_type_trait_object(&mut self, object: &'ast syn::TypeTraitObject) {
        self.record(Kind::Carrier, tokens(object));
        syn::visit::visit_type_trait_object(self, object);
    }

    fn visit_type_impl_trait(&mut self, ty: &'ast syn::TypeImplTrait) {
        if self.in_inputs {
            self.record(Kind::Carrier, tokens(ty));
        }
        syn::visit::visit_type_impl_trait(self, ty);
    }

    fn visit_type_bare_fn(&mut self, ty: &'ast syn::TypeBareFn) {
        self.record(Kind::Carrier, tokens(ty));
        syn::visit::visit_type_bare_fn(self, ty);
    }

    fn visit_type_param(&mut self, param: &'ast syn::TypeParam) {
        self.record(Kind::Carrier, format!("type parameter {}", param.ident));
        syn::visit::visit_type_param(self, param);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        let path = tokens(&mac.path);
        self.record(Kind::Macro(path.clone()), format!("`{path}!`"));
        self.macro_tokens(mac.tokens.clone());
        syn::visit::visit_macro(self, mac);
    }
}
