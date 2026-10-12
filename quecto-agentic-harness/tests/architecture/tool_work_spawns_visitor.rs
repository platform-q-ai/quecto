//! The spawn check's reader (`tool_work_spawns`): a syn visitor that
//! finds every spawn site of a file, by function.

use std::collections::BTreeMap;

use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use syn::visit::Visit;

/// Words that spawn, block on or enter a runtime wherever they are named.
const SPAWN_WORDS: &[&str] = &[
    "spawn_blocking",
    "spawn_local",
    "spawn_ok",
    "spawn_on",
    "spawn_blocking_on",
    "spawn_local_on",
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

/// What a receiver's words name when it is a spawner by name: a spawner
/// type, or the names such values go by.
const SPAWNER_NAMES: &[&str] = &["handle", "runtime", "builder", "thread"];

/// The crates and modules that spawn: renaming or globbing one hides the
/// spawns after it.
const SPAWNING_MODULES: &[&[&str]] = &[
    &["tokio"],
    &["tokio", "task"],
    &["tokio", "runtime"],
    &["std", "thread"],
    &["rayon"],
];

/// The words of `tokens`, groups flattened.
fn token_words(tokens: TokenStream) -> Vec<String> {
    let mut words = vec![];
    for tree in tokens {
        match tree {
            TokenTree::Group(group) => words.extend(token_words(group.stream())),
            TokenTree::Ident(ident) => words.push(ident.to_string()),
            TokenTree::Punct(_) | TokenTree::Literal(_) => {}
        }
    }
    words
}

/// The words of `expr`'s own path to its value, never its call arguments:
/// a path's segments (a qualified self's type too), a field's base and
/// name, a method call's receiver and method, a call's function. Any other
/// expression gives all its words, as it may hold a spawner anywhere.
fn spine(expr: &syn::Expr, out: &mut Vec<String>) {
    match expr {
        syn::Expr::Path(path) => {
            if let Some(qself) = &path.qself {
                out.extend(token_words(qself.ty.to_token_stream()));
            }
            out.extend(words(&path.path));
        }
        syn::Expr::Field(field) => {
            spine(&field.base, out);
            if let syn::Member::Named(name) = &field.member {
                out.push(name.to_string());
            }
        }
        syn::Expr::MethodCall(call) => {
            spine(&call.receiver, out);
            out.push(call.method.to_string());
        }
        syn::Expr::Call(call) => spine(&call.func, out),
        syn::Expr::Reference(inner) => spine(&inner.expr, out),
        syn::Expr::Paren(inner) => spine(&inner.expr, out),
        syn::Expr::Group(inner) => spine(&inner.expr, out),
        syn::Expr::Try(inner) => spine(&inner.expr, out),
        syn::Expr::Await(inner) => spine(&inner.base, out),
        syn::Expr::Unary(inner) => spine(&inner.expr, out),
        syn::Expr::Cast(inner) => spine(&inner.expr, out),
        other => out.extend(token_words(other.to_token_stream())),
    }
}

/// The names a pattern binds.
fn bound_names(pat: &syn::Pat, out: &mut Vec<String>) {
    match pat {
        syn::Pat::Ident(ident) => out.push(ident.ident.to_string()),
        syn::Pat::Type(typed) => bound_names(&typed.pat, out),
        syn::Pat::Reference(reference) => bound_names(&reference.pat, out),
        syn::Pat::Tuple(tuple) => tuple.elems.iter().for_each(|pat| bound_names(pat, out)),
        syn::Pat::TupleStruct(tuple) => tuple.elems.iter().for_each(|pat| bound_names(pat, out)),
        syn::Pat::Struct(fields) => fields
            .fields
            .iter()
            .for_each(|field| bound_names(&field.pat, out)),
        syn::Pat::Paren(paren) => bound_names(&paren.pat, out),
        syn::Pat::Or(or) => or.cases.iter().for_each(|pat| bound_names(pat, out)),
        syn::Pat::Slice(slice) => slice.elems.iter().for_each(|pat| bound_names(pat, out)),
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
    /// Per scope, the locals bound in it: whether each holds a spawner.
    locals: Vec<BTreeMap<String, bool>>,
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
        self.locals.push(BTreeMap::new());
        visit(self);
        self.locals.pop();
        self.scope.pop();
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
        bound_names(pat, &mut names);
        if let Some(locals) = self.locals.last_mut() {
            for name in names {
                locals.insert(name, spawner);
            }
        }
    }

    /// Whether `words` (a receiver's or a value's) name a spawner: a local
    /// bound to one, or a spawner's type or name that is not a local bound
    /// to something else.
    fn names_spawner(&self, words: &[String]) -> bool {
        words.iter().any(|word| match self.local(word) {
            Some(spawner) => spawner,
            None => {
                SPAWNER_TYPES.contains(&word.as_str()) || SPAWNER_NAMES.contains(&word.as_str())
            }
        })
    }

    /// Whether `expr`'s own path ([`spine`]) names a spawner.
    fn expr_names_spawner(&self, expr: &syn::Expr) -> bool {
        let mut words = vec![];
        spine(expr, &mut words);
        self.names_spawner(&words)
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
                    let word = ident.to_string();
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
        .map(|segment| segment.ident.to_string())
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
            prefix.push(path.ident.to_string());
            use_leaves(&path.tree, prefix, out);
            prefix.pop();
        }
        syn::UseTree::Name(name) => out.push((
            [prefix.clone(), vec![name.ident.to_string()]].concat(),
            false,
        )),
        syn::UseTree::Rename(rename) => {
            // `self as t` renames the prefix itself.
            let path = match rename.ident == "self" {
                true => prefix.clone(),
                false => [prefix.clone(), vec![rename.ident.to_string()]].concat(),
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
        let name = vec![item.ident.to_string()];
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

    fn visit_local(&mut self, local: &'ast syn::Local) {
        let typed = declared_type(&local.pat).is_some_and(names_spawner_type);
        let valued = local
            .init
            .as_ref()
            .is_some_and(|init| self.expr_names_spawner(&init.expr));
        // Visit the value first: it is read before the names are bound.
        syn::visit::visit_local(self, local);
        self.bind(&local.pat, typed || valued);
    }

    fn visit_expr_let(&mut self, expr: &'ast syn::ExprLet) {
        let spawner = self.expr_names_spawner(&expr.expr)
            || declared_type(&expr.pat).is_some_and(names_spawner_type);
        self.bind(&expr.pat, spawner);
        syn::visit::visit_expr_let(self, expr);
    }

    fn visit_expr_match(&mut self, expr: &'ast syn::ExprMatch) {
        let spawner = self.expr_names_spawner(&expr.expr);
        for arm in &expr.arms {
            self.bind(&arm.pat, spawner);
        }
        syn::visit::visit_expr_match(self, expr);
    }

    fn visit_expr_closure(&mut self, closure: &'ast syn::ExprClosure) {
        // A typed parameter is what its type says; an untyped one keeps
        // whatever the method it is passed to bound it as.
        for input in &closure.inputs {
            if let Some(ty) = declared_type(input) {
                self.bind(input, names_spawner_type(ty));
            }
        }
        syn::visit::visit_expr_closure(self, closure);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let method = call.method.to_string();
        let receiver = call.receiver.to_token_stream().to_string();
        let on_spawner = self.expr_names_spawner(&call.receiver);
        let spawns = SPAWNED_BY_SPAWNER.contains(&method.as_str()) && on_spawner;
        if SPAWN_WORDS.contains(&method.as_str()) || spawns {
            self.record(format!("`.{method}(..)` on `{receiver}`"));
        }
        // A closure handed to a spawner's method gets the spawner (or
        // what it gives) as its untyped parameters.
        if on_spawner {
            for arg in &call.args {
                if let syn::Expr::Closure(closure) = arg {
                    for input in &closure.inputs {
                        self.bind(input, true);
                    }
                }
            }
        }
        syn::visit::visit_expr_method_call(self, call);
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
            found: vec![],
        };
        sites.visit_file(parsed);
        found.extend(sites.found);
    }
    found
}
