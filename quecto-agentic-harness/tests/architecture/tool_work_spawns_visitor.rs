//! The spawn check's reader (`tool_work_spawns`): a syn visitor that
//! finds every spawn site of a file, by function.

use std::collections::BTreeMap;

use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use syn::ext::IdentExt;
use syn::visit::Visit;

/// Words that spawn, block on or enter a runtime wherever they are named.
const SPAWN_WORDS: &[&str] = &[
    "spawn_blocking",
    "spawn_local",
    "spawn_ok",
    "spawn_on",
    "spawn_blocking_on",
    "spawn_local_on",
    "spawn_scoped",
    "spawn_unchecked",
    "block_on",
    "block_in_place",
];

/// The path segments before `spawn`, `scope` or `enter` that make it a
/// task or thread spawn or a runtime entered (`tokio::spawn`,
/// `tokio::task::spawn`, `std::thread::spawn`, `std::thread::scope`,
/// `Handle::spawn`, `JoinSet::spawn`, `rayon::spawn`), as a process's
/// `Command::spawn` is not.
const SPAWNERS: &[&str] = &[
    "tokio", "task", "thread", "Handle", "Runtime", "JoinSet", "rayon",
];

/// The words after a spawner, or methods on a spawning receiver, that
/// spawn or enter a runtime: `scope` runs scoped threads.
const SPAWNED_BY_SPAWNER: &[&str] = &["spawn", "scope", "enter"];

/// The types that spawn: a runtime, its handle, a join set, a thread
/// builder or scope, a thread pool.
const SPAWNER_TYPES: &[&str] = &[
    "Handle",
    "Runtime",
    "JoinSet",
    "Builder",
    "ThreadPool",
    "Scope",
];

/// The names a spawner goes by, as a path's base or a field: a value of
/// one of these names is taken for a spawner unless it is a local bound to
/// something else.
const SPAWNER_NAMES: &[&str] = &["handle", "runtime", "builder", "thread"];

/// The methods that answer a spawner (`runtime.handle()`).
const SPAWNER_METHODS: &[&str] = &["handle"];

/// The crates and modules that spawn: renaming or globbing one hides the
/// spawns after it.
const SPAWNING_MODULES: &[&[&str]] = &[
    &["tokio"],
    &["tokio", "task"],
    &["tokio", "runtime"],
    &["std", "thread"],
    &["rayon"],
];

/// An identifier as written, `r#` taken off.
fn word(ident: &proc_macro2::Ident) -> String {
    ident.unraw().to_string()
}

/// The words of `tokens`, groups flattened, raw identifiers unrawed.
fn token_words(tokens: TokenStream) -> Vec<String> {
    let mut words = vec![];
    for tree in tokens {
        match tree {
            TokenTree::Group(group) => words.extend(token_words(group.stream())),
            TokenTree::Ident(ident) => words.push(word(&ident)),
            TokenTree::Punct(_) | TokenTree::Literal(_) => {}
        }
    }
    words
}

/// One part of an expression's own path to its value.
enum Part {
    /// The first segment of a path: a local, or a type or module.
    Base(String),
    /// A later segment of a path (`Handle` in `tokio::runtime::Handle`).
    Segment(String),
    /// A field (`handle` in `self.handle`).
    Field(String),
    /// A method called on the value so far.
    Method(String),
}

/// `expr`'s own path to its value ([`Part`]s): a path's segments (a
/// qualified self's type too), a field's base and name, a method call's
/// receiver and method, a call's function; and, apart, the words of its
/// call arguments and of any other expression (`args`).
fn spine(expr: &syn::Expr, parts: &mut Vec<Part>, args: &mut Vec<String>) {
    match expr {
        syn::Expr::Path(path) => {
            if let Some(qself) = &path.qself {
                parts.extend(
                    token_words(qself.ty.to_token_stream())
                        .into_iter()
                        .map(Part::Segment),
                );
            }
            for (index, segment) in words(&path.path).into_iter().enumerate() {
                parts.push(match (index, &path.qself) {
                    (0, None) => Part::Base(segment),
                    (_, _) => Part::Segment(segment),
                });
            }
        }
        syn::Expr::Field(field) => {
            spine(&field.base, parts, args);
            if let syn::Member::Named(name) = &field.member {
                parts.push(Part::Field(word(name)));
            }
        }
        syn::Expr::MethodCall(call) => {
            spine(&call.receiver, parts, args);
            parts.push(Part::Method(word(&call.method)));
            for arg in &call.args {
                args.extend(token_words(arg.to_token_stream()));
            }
        }
        syn::Expr::Call(call) => {
            spine(&call.func, parts, args);
            for arg in &call.args {
                args.extend(token_words(arg.to_token_stream()));
            }
        }
        syn::Expr::Reference(inner) => spine(&inner.expr, parts, args),
        syn::Expr::Paren(inner) => spine(&inner.expr, parts, args),
        syn::Expr::Group(inner) => spine(&inner.expr, parts, args),
        syn::Expr::Try(inner) => spine(&inner.expr, parts, args),
        syn::Expr::Await(inner) => spine(&inner.base, parts, args),
        syn::Expr::Unary(inner) => spine(&inner.expr, parts, args),
        syn::Expr::Cast(inner) => spine(&inner.expr, parts, args),
        other => args.extend(token_words(other.to_token_stream())),
    }
}

/// The names a pattern binds, each with whether it holds a spawner:
/// `spawner` for the whole value, or the binding's own type, or a struct
/// field that names one (`S { handle, .. }`).
fn bound_names(pat: &syn::Pat, spawner: bool, out: &mut Vec<(String, bool)>) {
    match pat {
        syn::Pat::Ident(ident) => out.push((word(&ident.ident), spawner)),
        syn::Pat::Type(typed) => {
            bound_names(&typed.pat, spawner || names_spawner_type(&typed.ty), out)
        }
        syn::Pat::Reference(reference) => bound_names(&reference.pat, spawner, out),
        syn::Pat::Tuple(tuple) => tuple
            .elems
            .iter()
            .for_each(|pat| bound_names(pat, spawner, out)),
        syn::Pat::TupleStruct(tuple) => tuple
            .elems
            .iter()
            .for_each(|pat| bound_names(pat, spawner, out)),
        syn::Pat::Struct(fields) => fields.fields.iter().for_each(|field| {
            let named = match &field.member {
                syn::Member::Named(name) => {
                    let name = word(name);
                    SPAWNER_NAMES.contains(&name.as_str()) || SPAWNER_TYPES.contains(&name.as_str())
                }
                syn::Member::Unnamed(_) => false,
            };
            bound_names(&field.pat, spawner || named, out)
        }),
        syn::Pat::Paren(paren) => bound_names(&paren.pat, spawner, out),
        syn::Pat::Or(or) => or
            .cases
            .iter()
            .for_each(|pat| bound_names(pat, spawner, out)),
        syn::Pat::Slice(slice) => slice
            .elems
            .iter()
            .for_each(|pat| bound_names(pat, spawner, out)),
        _ => {}
    }
}

/// Whether a type names a spawner.
fn names_spawner_type(ty: &syn::Type) -> bool {
    token_words(ty.to_token_stream())
        .iter()
        .any(|word| SPAWNER_TYPES.contains(&word.as_str()))
}

/// The type a pattern is declared with (`h: Handle`), if any.
fn declared_type(pat: &syn::Pat) -> Option<&syn::Type> {
    match pat {
        syn::Pat::Type(typed) => Some(&typed.ty),
        _ => None,
    }
}

struct Sites<'a> {
    file: &'a str,
    scope: Vec<String>,
    /// Per scope (a function, block, closure, arm or `if let`), the
    /// locals bound in it: whether each holds a spawner.
    locals: Vec<BTreeMap<String, bool>>,
    /// Set just before a closure handed to a spawner's method is visited:
    /// its untyped parameters are spawners.
    spawner_closure: bool,
    found: Vec<(String, String, String)>,
}

impl Sites<'_> {
    fn record(&mut self, what: String) {
        let site = match self.scope.is_empty() {
            true => "<module scope>".to_string(),
            false => self.scope.join("::"),
        };
        self.found.push((self.file.to_string(), site, what));
    }

    fn scoped(&mut self, name: String, visit: impl FnOnce(&mut Self)) {
        self.scope.push(name);
        self.with_locals(visit);
        self.scope.pop();
    }

    /// Visit with a scope of locals of its own: what is bound in it is
    /// gone after, never hiding an outer binding.
    fn with_locals(&mut self, visit: impl FnOnce(&mut Self)) {
        self.locals.push(BTreeMap::new());
        visit(self);
        self.locals.pop();
    }

    /// Whether `word` is a local, innermost scope first, and holds a
    /// spawner; `None` when it is no local this checker saw bound.
    fn local(&self, word: &str) -> Option<bool> {
        self.locals
            .iter()
            .rev()
            .find_map(|locals| locals.get(word).copied())
    }

    /// Bind `pat`'s names in the current scope, as spawners or not.
    fn bind(&mut self, pat: &syn::Pat, spawner: bool) {
        let mut names = vec![];
        bound_names(pat, spawner, &mut names);
        if let Some(locals) = self.locals.last_mut() {
            locals.extend(names);
        }
    }

    /// Assign to the local `name` (`h = Handle::current()` after `let h;`):
    /// where it is bound, or here when it is no local seen.
    fn assign(&mut self, name: String, spawner: bool) {
        let at = self
            .locals
            .iter()
            .rposition(|locals| locals.contains_key(&name))
            .unwrap_or(self.locals.len() - 1);
        self.locals[at].insert(name, spawner);
    }

    /// Whether macro tokens, as words, name a spawner: a local bound to
    /// one, or a spawner's type or name that is not a local bound to
    /// something else.
    fn names_spawner(&self, words: &[String]) -> bool {
        words.iter().any(|word| match self.local(word) {
            Some(spawner) => spawner,
            None => {
                SPAWNER_TYPES.contains(&word.as_str()) || SPAWNER_NAMES.contains(&word.as_str())
            }
        })
    }

    /// Whether `expr` holds a spawner: its own path names one (a path's
    /// base only is looked up as a local; a later segment counts when it is
    /// a spawner type, a field when it is a spawner's type or name, a method
    /// when it answers a spawner), or its call arguments hold a spawner
    /// type or a local bound to one (`Arc::new(Handle::current())`,
    /// `identity(h)`), never by a mere name (`Command::new(runtime)`).
    fn holds_spawner(&self, expr: &syn::Expr) -> bool {
        let (mut parts, mut args) = (vec![], vec![]);
        spine(expr, &mut parts, &mut args);
        let is_type = |word: &str| SPAWNER_TYPES.contains(&word);
        let is_name = |word: &str| SPAWNER_NAMES.contains(&word);
        parts.iter().any(|part| match part {
            Part::Base(word) => match self.local(word) {
                Some(spawner) => spawner,
                None => is_type(word) || is_name(word),
            },
            Part::Segment(word) => is_type(word),
            Part::Field(word) => is_type(word) || is_name(word),
            Part::Method(word) => SPAWNER_METHODS.contains(&word.as_str()),
        }) || args
            .iter()
            .any(|word| is_type(word) || self.local(word) == Some(true))
    }

    /// Bind a function's typed parameters by their types.
    fn bind_params<'p>(&mut self, inputs: impl Iterator<Item = &'p syn::FnArg>) {
        for input in inputs {
            if let syn::FnArg::Typed(typed) = input {
                self.bind(&typed.pat, names_spawner_type(&typed.ty));
            }
        }
    }

    /// A path that names a spawn form: a spawn word anywhere, or `spawn`,
    /// `scope` or `enter` after a spawner.
    fn path(&mut self, words: &[String]) {
        let spawn_word = words
            .iter()
            .any(|word| SPAWN_WORDS.contains(&word.as_str()));
        let spawn_after_spawner = words.iter().enumerate().any(|(index, word)| {
            SPAWNED_BY_SPAWNER.contains(&word.as_str())
                && index > 0
                && words[..index]
                    .iter()
                    .any(|before| SPAWNERS.contains(&before.as_str()))
        });
        if spawn_word || spawn_after_spawner {
            self.record(format!("`{}`", words.join("::")));
        }
    }

    /// A macro's tokens held to the same rules: a spawn word, `spawn`,
    /// `scope` or `enter` after a spawner and `::` (past a turbofish), or
    /// `.spawn`, `.scope` or `.enter` on a receiver whose words name a
    /// spawner (back to the start of its expression). A group stands in
    /// `seen` as one marker, so a lookback never joins tokens across it
    /// and never reads a call's arguments.
    fn tokens(&mut self, stream: TokenStream) {
        let mut seen: Vec<String> = vec![];
        for tree in stream {
            match &tree {
                TokenTree::Group(group) => {
                    self.tokens(group.stream());
                    seen.push("<group>".to_string());
                }
                TokenTree::Ident(ident) => {
                    let word = word(ident);
                    let spawns = SPAWNED_BY_SPAWNER.contains(&word.as_str());
                    let by_path = spawns
                        && segment_before(&seen)
                            .is_some_and(|before| SPAWNERS.contains(&before.as_str()));
                    let by_method = spawns
                        && seen.last().is_some_and(|before| before == ".")
                        && self.names_spawner(
                            &seen
                                .iter()
                                .rev()
                                .take_while(|token| !matches!(token.as_str(), ";" | "," | "="))
                                .cloned()
                                .collect::<Vec<_>>(),
                        );
                    if SPAWN_WORDS.contains(&word.as_str()) || by_path || by_method {
                        self.record(format!("`{word}` in a macro"));
                    }
                    seen.push(word);
                }
                TokenTree::Punct(punct) => seen.push(punct.as_char().to_string()),
                TokenTree::Literal(_) => seen.push(String::new()),
            }
        }
    }
}

/// The path segment before a trailing `::` in `seen`, past a turbofish's
/// `<..>` (`JoinSet::<()>::`): `None` when `seen` does not end in `::`.
fn segment_before(seen: &[String]) -> Option<String> {
    let mut at = seen.len().checked_sub(2)?;
    if seen[at..] != [":".to_string(), ":".to_string()] {
        return None;
    }
    // Past a turbofish: from its `>` back to its matching `<`, then `::`.
    if at > 0 && seen[at - 1] == ">" {
        let mut depth = 0usize;
        let mut index = at;
        loop {
            index = index.checked_sub(1)?;
            match seen[index].as_str() {
                ">" => depth += 1,
                "<" => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
        at = index.checked_sub(2)?;
        if seen[at..index] != [":".to_string(), ":".to_string()] {
            return None;
        }
    }
    at.checked_sub(1).map(|before| seen[before].clone())
}

fn is_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|attr| {
        attr.path().is_ident("test")
            || (attr.path().is_ident("cfg")
                && attr.meta.to_token_stream().to_string() == "cfg (test)")
    })
}

fn words(path: &syn::Path) -> Vec<String> {
    path.segments
        .iter()
        .map(|segment| word(&segment.ident))
        .collect()
}

/// A `use` leaf: its path, and whether it hides a spawn: it renames a path
/// ending in a spawner or spawning module, or globs a spawning module.
type UseLeaf = (Vec<String>, bool);

/// Whether `path` is a spawning module.
fn spawning_module(path: &[String]) -> bool {
    SPAWNING_MODULES
        .iter()
        .any(|module| module.iter().copied().eq(path.iter().map(String::as_str)))
}

/// Whether renaming `path` hides a spawn: it is a spawning module, or ends
/// in a spawner or a spawning module's name.
fn hidden_by_rename(path: &[String]) -> bool {
    spawning_module(path)
        || path.last().is_some_and(|last| {
            SPAWNERS.contains(&last.as_str()) || SPAWNER_TYPES.contains(&last.as_str())
        })
}

/// The leaves of a `use` tree, each with the path before it.
fn use_leaves(tree: &syn::UseTree, prefix: &mut Vec<String>, out: &mut Vec<UseLeaf>) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(word(&path.ident));
            use_leaves(&path.tree, prefix, out);
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            out.push(([prefix.clone(), vec![word(&name.ident)]].concat(), false))
        }
        syn::UseTree::Rename(rename) => {
            // `self as t` renames the prefix itself.
            let path = match rename.ident == "self" {
                true => prefix.clone(),
                false => [prefix.clone(), vec![word(&rename.ident)]].concat(),
            };
            let hides = hidden_by_rename(&path);
            out.push((path, hides));
        }
        syn::UseTree::Group(group) => {
            for tree in &group.items {
                use_leaves(tree, prefix, out);
            }
        }
        syn::UseTree::Glob(_) => out.push((prefix.clone(), spawning_module(prefix))),
    }
}

impl<'ast> Visit<'ast> for Sites<'_> {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        if !is_test(&item.attrs) {
            self.scoped(format!("mod {}", item.ident), |sites| {
                syn::visit::visit_item_mod(sites, item)
            });
        }
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if !is_test(&item.attrs) {
            self.scoped(format!("fn {}", item.sig.ident), |sites| {
                sites.bind_params(item.sig.inputs.iter());
                syn::visit::visit_item_fn(sites, item)
            });
        }
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if !is_test(&item.attrs) {
            let name = format!(
                "impl {}",
                item.self_ty.to_token_stream().to_string().replace(' ', "")
            );
            self.scoped(name, |sites| syn::visit::visit_item_impl(sites, item));
        }
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        self.scoped(format!("trait {}", item.ident), |sites| {
            syn::visit::visit_item_trait(sites, item)
        });
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if !is_test(&item.attrs) {
            self.scoped(item.sig.ident.to_string(), |sites| {
                sites.bind_params(item.sig.inputs.iter());
                syn::visit::visit_impl_item_fn(sites, item)
            });
        }
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        self.scoped(item.sig.ident.to_string(), |sites| {
            sites.bind_params(item.sig.inputs.iter());
            syn::visit::visit_trait_item_fn(sites, item)
        });
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        let mut leaves = vec![];
        use_leaves(&item.tree, &mut vec![], &mut leaves);
        for (leaf, hides) in leaves {
            match hides {
                true => self.record(format!(
                    "a `use` renaming or globbing `{}`",
                    leaf.join("::")
                )),
                false => self.path(&leaf),
            }
        }
    }

    fn visit_item_extern_crate(&mut self, item: &'ast syn::ItemExternCrate) {
        let name = vec![word(&item.ident)];
        if item.rename.is_some() && hidden_by_rename(&name) {
            self.record(format!("`extern crate {}` renamed", item.ident));
        }
        syn::visit::visit_item_extern_crate(self, item);
    }

    fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
        if names_spawner_type(&item.ty) {
            self.record(format!("`type {}`, an alias of a spawner", item.ident));
        }
        syn::visit::visit_item_type(self, item);
    }

    fn visit_item_static(&mut self, item: &'ast syn::ItemStatic) {
        if names_spawner_type(&item.ty) {
            self.record(format!("`static {}` of a spawner type", item.ident));
        }
        syn::visit::visit_item_static(self, item);
    }

    fn visit_item_const(&mut self, item: &'ast syn::ItemConst) {
        if names_spawner_type(&item.ty) {
            self.record(format!("`const {}` of a spawner type", item.ident));
        }
        syn::visit::visit_item_const(self, item);
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.path(&words(path));
        syn::visit::visit_path(self, path);
    }

    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        // `<Handle>::spawn`: the qualified self's type, then the path.
        if let Some(qself) = &path.qself {
            let mut all = token_words(qself.ty.to_token_stream());
            all.extend(words(&path.path));
            self.path(&all);
        }
        syn::visit::visit_expr_path(self, path);
    }

    fn visit_block(&mut self, block: &'ast syn::Block) {
        self.with_locals(|sites| syn::visit::visit_block(sites, block));
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        let valued = local
            .init
            .as_ref()
            .is_some_and(|init| self.holds_spawner(&init.expr));
        // Visit the value first: it is read before the names are bound.
        syn::visit::visit_local(self, local);
        self.bind(&local.pat, valued);
    }

    fn visit_expr_assign(&mut self, assign: &'ast syn::ExprAssign) {
        syn::visit::visit_expr_assign(self, assign);
        if let syn::Expr::Path(path) = &*assign.left
            && path.qself.is_none()
            && path.path.segments.len() == 1
        {
            let spawner = self.holds_spawner(&assign.right);
            self.assign(word(&path.path.segments[0].ident), spawner);
        }
    }

    fn visit_expr_let(&mut self, expr: &'ast syn::ExprLet) {
        let spawner = self.holds_spawner(&expr.expr);
        syn::visit::visit_expr_let(self, expr);
        self.bind(&expr.pat, spawner);
    }

    fn visit_expr_if(&mut self, expr: &'ast syn::ExprIf) {
        // An `if let`'s names live only in the `if`.
        self.with_locals(|sites| syn::visit::visit_expr_if(sites, expr));
    }

    fn visit_expr_while(&mut self, expr: &'ast syn::ExprWhile) {
        self.with_locals(|sites| syn::visit::visit_expr_while(sites, expr));
    }

    fn visit_expr_for_loop(&mut self, expr: &'ast syn::ExprForLoop) {
        self.visit_expr(&expr.expr);
        let spawner = self.holds_spawner(&expr.expr);
        self.with_locals(|sites| {
            sites.bind(&expr.pat, spawner);
            sites.visit_pat(&expr.pat);
            sites.visit_block(&expr.body);
        });
    }

    fn visit_expr_match(&mut self, expr: &'ast syn::ExprMatch) {
        self.visit_expr(&expr.expr);
        let spawner = self.holds_spawner(&expr.expr);
        for arm in &expr.arms {
            self.with_locals(|sites| {
                sites.bind(&arm.pat, spawner);
                sites.visit_arm(arm);
            });
        }
    }

    fn visit_expr_closure(&mut self, closure: &'ast syn::ExprClosure) {
        // A typed parameter is what its type says; an untyped one is a
        // spawner when the closure is handed to a spawner's method.
        let handed = std::mem::take(&mut self.spawner_closure);
        self.with_locals(|sites| {
            for input in &closure.inputs {
                sites.bind(input, handed && declared_type(input).is_none());
            }
            syn::visit::visit_expr_closure(sites, closure);
        });
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let method = word(&call.method);
        let receiver = call.receiver.to_token_stream().to_string();
        let on_spawner = self.holds_spawner(&call.receiver);
        let spawns = SPAWNED_BY_SPAWNER.contains(&method.as_str()) && on_spawner;
        if SPAWN_WORDS.contains(&method.as_str()) || spawns {
            self.record(format!("`.{method}(..)` on `{receiver}`"));
        }
        self.visit_expr(&call.receiver);
        if let Some(turbofish) = &call.turbofish {
            self.visit_angle_bracketed_generic_arguments(turbofish);
        }
        for arg in &call.args {
            // A closure handed to a spawner's method gets the spawner (or
            // what it gives) as its untyped parameters.
            self.spawner_closure = on_spawner && matches!(arg, syn::Expr::Closure(_));
            self.visit_expr(arg);
            self.spawner_closure = false;
        }
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        self.tokens(mac.tokens.clone());
        syn::visit::visit_macro(self, mac);
    }
}

/// Every spawn site of `files`: (file, site, what).
pub(super) fn spawn_sites(files: &[(String, syn::File)]) -> Vec<(String, String, String)> {
    let mut found = vec![];
    for (path, parsed) in files {
        let mut sites = Sites {
            file: path,
            scope: vec![],
            locals: vec![BTreeMap::new()],
            spawner_closure: false,
            found: vec![],
        };
        sites.visit_file(parsed);
        found.extend(sites.found);
    }
    found
}
