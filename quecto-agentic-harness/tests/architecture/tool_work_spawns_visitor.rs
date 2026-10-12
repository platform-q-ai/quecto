//! The spawn check's reader (`tool_work_spawns`): a syn visitor that
//! finds every spawn site of a file, by function. What a value holds is
//! read by [`super::values`].

use std::collections::{BTreeMap, BTreeSet};

use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use syn::visit::Visit;

use super::values::{
    State, bound_names, is_name, is_type, names_spawner_type, token_words, type_state, word,
};

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
    "spawn_with_handle",
    "spawn_obj",
    "spawn_local_obj",
    "block_on",
    "block_on_stream",
    "block_in_place",
];

/// The path segments before `spawn`, `scope` or `enter` that make it a
/// task or thread spawn or a runtime entered (`tokio::spawn`,
/// `tokio::task::spawn`, `std::thread::spawn`, `std::thread::scope`,
/// `Handle::spawn`, `JoinSet::spawn`, `rayon::spawn`, `smol::spawn`,
/// `TaskTracker::spawn`), as a process's `Command::spawn` is not.
pub(super) const SPAWNERS: &[&str] = &[
    "tokio",
    "task",
    "thread",
    "Handle",
    "Runtime",
    "JoinSet",
    "rayon",
    "smol",
    "async_std",
    "TaskTracker",
];

/// The words after a spawner, or methods on a spawning receiver, that
/// spawn or enter a runtime: `scope` runs scoped threads.
const SPAWNED_BY_SPAWNER: &[&str] = &["spawn", "scope", "enter"];

/// The crates and modules that spawn: renaming or globbing one hides the
/// spawns after it.
const SPAWNING_MODULES: &[&[&str]] = &[
    &["tokio"],
    &["tokio", "task"],
    &["tokio", "runtime"],
    &["std", "thread"],
    &["rayon"],
    &["smol"],
    &["async_std"],
];

/// The generic parameters of a function or impl, which no concrete type
/// names.
fn generic_names(generics: &syn::Generics) -> BTreeSet<String> {
    generics
        .type_params()
        .map(|param| word(&param.ident))
        .collect()
}

struct Sites<'a> {
    file: &'a str,
    scope: Vec<String>,
    /// Per scope (a function, block, closure, arm or `if`), the locals
    /// bound in it and what each holds.
    locals: Vec<BTreeMap<String, State>>,
    /// Per item scope, every name ever bound to a spawner in it: such a
    /// name stays a spawner for the rest of the item, as the merged check
    /// read it.
    ever: Vec<BTreeSet<String>>,
    /// Per item scope, the generic parameters in force.
    generics: Vec<BTreeSet<String>>,
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

    fn scoped(&mut self, name: String, generics: BTreeSet<String>, visit: impl FnOnce(&mut Self)) {
        self.scope.push(name);
        self.ever.push(BTreeSet::new());
        self.generics.push(generics);
        self.with_locals(visit);
        self.generics.pop();
        self.ever.pop();
        self.scope.pop();
    }

    /// Visit with a scope of locals of its own: what is bound in it is
    /// gone after, never hiding an outer binding.
    fn with_locals(&mut self, visit: impl FnOnce(&mut Self)) {
        self.locals.push(BTreeMap::new());
        visit(self);
        self.locals.pop();
    }

    /// What the local `word` holds, innermost scope first; `None` when it
    /// is no local this checker saw bound.
    fn local(&self, word: &str) -> Option<State> {
        self.locals
            .iter()
            .rev()
            .find_map(|locals| locals.get(word).copied())
    }

    /// Whether `word` was ever bound to a spawner in this item.
    fn ever_spawner(&self, word: &str) -> bool {
        self.ever.iter().any(|names| names.contains(word))
    }

    /// Whether `word` is a generic parameter in force.
    fn generic(&self, word: &str) -> bool {
        self.generics.iter().any(|names| names.contains(word))
    }

    /// Bind `pat`'s names in the current scope, each with what it holds.
    fn bind(&mut self, pat: &syn::Pat, state: State) {
        let mut names = vec![];
        bound_names(
            pat,
            state,
            &|ty| type_state(ty, &|word| self.generic(word)),
            &mut names,
        );
        for (name, state) in names {
            if state == State::Spawner {
                if let Some(ever) = self.ever.last_mut() {
                    ever.insert(name.clone());
                }
            }
            assert!(!self.locals.is_empty(), "a scope of locals is open");
            if let Some(locals) = self.locals.last_mut() {
                locals.insert(name, state);
            }
        }
    }

    /// Assign to the local `name` (`h = Handle::current()` after `let h;`):
    /// what it holds only ever rises, where it is bound, or here when it is
    /// no local seen.
    fn assign(&mut self, name: String, state: State) {
        assert!(!self.locals.is_empty(), "a scope of locals is open");
        let at = self
            .locals
            .iter()
            .rposition(|locals| locals.contains_key(&name))
            .unwrap_or(self.locals.len() - 1);
        let raised = self.locals[at]
            .get(&name)
            .map_or(state, |old| (*old).max(state));
        if raised == State::Spawner {
            if let Some(ever) = self.ever.last_mut() {
                ever.insert(name.clone());
            }
        }
        self.locals[at].insert(name, raised);
    }

    /// Whether the word `word` of a value makes it a spawner. `base`: it is
    /// a path's first segment, the only kind looked up as a local; `prev`:
    /// the word before it. A name ever bound to a spawner in this item is
    /// one; a local known to hold no spawner is none whatever its name;
    /// otherwise a spawner type or name is one, but `builder` after a type
    /// that is no spawner (`Client::builder()`).
    fn word_spawns(&self, word: &str, base: bool, prev: Option<&str>) -> bool {
        if self.ever_spawner(word) {
            return true;
        }
        match (base, self.local(word)) {
            (true, Some(State::Spawner)) => return true,
            (true, Some(State::KnownNot)) => return false,
            (true, Some(State::Unknown) | None) | (false, _) => {}
        }
        let builder_of_another = word == "builder"
            && prev.is_some_and(|prev| {
                prev.starts_with(|c: char| c.is_ascii_uppercase()) && !is_type(prev)
            });
        is_type(word) || (is_name(word) && !builder_of_another)
    }

    /// Whether `expr` holds a spawner: any of its words makes it one
    /// ([`Self::word_spawns`]), its paths' first segments looked up as
    /// locals.
    fn holds_spawner(&self, expr: &syn::Expr) -> bool {
        let mut bases = super::values::Bases::default();
        bases.visit_expr(expr);
        let words = token_words(expr.to_token_stream());
        words.iter().enumerate().any(|(index, word)| {
            let prev = index.checked_sub(1).map(|at| words[at].as_str());
            self.word_spawns(word, bases.0.contains(word), prev)
        })
    }

    /// What `expr` holds: a spawner ([`Self::holds_spawner`]), none
    /// ([`super::values::known_not`]), or not known.
    fn value_state(&self, expr: &syn::Expr) -> State {
        match (
            self.holds_spawner(expr),
            super::values::known_not(expr, &|word| self.local(word), &|word| self.generic(word)),
        ) {
            (true, _) => State::Spawner,
            (false, true) => State::KnownNot,
            (false, false) => State::Unknown,
        }
    }

    /// Bind a function's typed parameters by their types.
    fn bind_params<'p>(&mut self, inputs: impl Iterator<Item = &'p syn::FnArg>) {
        for input in inputs {
            match input {
                syn::FnArg::Typed(typed) => {
                    let state = type_state(&typed.ty, &|word| self.generic(word));
                    self.bind(&typed.pat, state);
                }
                syn::FnArg::Receiver(_) => {}
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
    /// `.spawn`, `.scope` or `.enter` on a receiver whose words make it a
    /// spawner (back to the start of its expression, whose first word is
    /// its base). A group stands in `seen` as one marker, so a lookback
    /// never joins tokens across it and never reads a call's arguments.
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
                        && self.receiver_tokens_spawn(&seen);
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

    /// Whether the receiver ending `seen` (back to `;`, `,` or `=`) makes a
    /// spawner: its words in order, the first its base.
    fn receiver_tokens_spawn(&self, seen: &[String]) -> bool {
        let start = seen
            .iter()
            .rposition(|token| matches!(token.as_str(), ";" | "," | "="))
            .map_or(0, |at| at + 1);
        let words: Vec<&String> = seen[start..]
            .iter()
            .filter(|token| {
                token
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_alphanumeric() || c == '_')
            })
            .collect();
        words.iter().enumerate().any(|(index, word)| {
            let prev = index.checked_sub(1).map(|at| words[at].as_str());
            self.word_spawns(word, index == 0, prev)
        })
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
        || path
            .last()
            .is_some_and(|last| SPAWNERS.contains(&last.as_str()) || is_type(last))
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
            self.scoped(format!("mod {}", item.ident), BTreeSet::new(), |sites| {
                syn::visit::visit_item_mod(sites, item)
            });
        }
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if !is_test(&item.attrs) {
            let generics = generic_names(&item.sig.generics);
            self.scoped(format!("fn {}", item.sig.ident), generics, |sites| {
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
            let generics = generic_names(&item.generics);
            self.scoped(name, generics, |sites| {
                syn::visit::visit_item_impl(sites, item)
            });
        }
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        let generics = generic_names(&item.generics);
        self.scoped(format!("trait {}", item.ident), generics, |sites| {
            syn::visit::visit_item_trait(sites, item)
        });
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        if !is_test(&item.attrs) {
            let generics = generic_names(&item.sig.generics);
            self.scoped(item.sig.ident.to_string(), generics, |sites| {
                sites.bind_params(item.sig.inputs.iter());
                syn::visit::visit_impl_item_fn(sites, item)
            });
        }
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        let generics = generic_names(&item.sig.generics);
        self.scoped(item.sig.ident.to_string(), generics, |sites| {
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

    fn visit_impl_item_type(&mut self, item: &'ast syn::ImplItemType) {
        if names_spawner_type(&item.ty) {
            self.record(format!("`type {}`, an alias of a spawner", item.ident));
        }
        syn::visit::visit_impl_item_type(self, item);
    }

    fn visit_trait_item_type(&mut self, item: &'ast syn::TraitItemType) {
        if item
            .default
            .as_ref()
            .is_some_and(|(_, ty)| names_spawner_type(ty))
        {
            self.record(format!("`type {}`, an alias of a spawner", item.ident));
        }
        syn::visit::visit_trait_item_type(self, item);
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

    fn visit_impl_item_const(&mut self, item: &'ast syn::ImplItemConst) {
        if names_spawner_type(&item.ty) {
            self.record(format!("`const {}` of a spawner type", item.ident));
        }
        syn::visit::visit_impl_item_const(self, item);
    }

    fn visit_trait_item_const(&mut self, item: &'ast syn::TraitItemConst) {
        if names_spawner_type(&item.ty) {
            self.record(format!("`const {}` of a spawner type", item.ident));
        }
        syn::visit::visit_trait_item_const(self, item);
    }

    fn visit_type_param(&mut self, param: &'ast syn::TypeParam) {
        if param.default.as_ref().is_some_and(names_spawner_type) {
            self.record(format!(
                "`{}`, a generic defaulting to a spawner",
                param.ident
            ));
        }
        syn::visit::visit_type_param(self, param);
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
        // No value (`let h;`): not known until it is assigned.
        let state = local
            .init
            .as_ref()
            .map_or(State::Unknown, |init| self.value_state(&init.expr));
        // Visit the value first: it is read before the names are bound.
        syn::visit::visit_local(self, local);
        self.bind(&local.pat, state);
    }

    fn visit_expr_assign(&mut self, assign: &'ast syn::ExprAssign) {
        syn::visit::visit_expr_assign(self, assign);
        if let syn::Expr::Path(path) = &*assign.left
            && path.qself.is_none()
            && path.path.segments.len() == 1
        {
            let state = self.value_state(&assign.right);
            self.assign(word(&path.path.segments[0].ident), state);
        }
    }

    fn visit_expr_let(&mut self, expr: &'ast syn::ExprLet) {
        let state = self.value_state(&expr.expr);
        syn::visit::visit_expr_let(self, expr);
        self.bind(&expr.pat, state);
    }

    fn visit_expr_if(&mut self, expr: &'ast syn::ExprIf) {
        // An `if let`'s names live in its condition and `then` branch only,
        // never its `else`.
        self.with_locals(|sites| {
            sites.visit_expr(&expr.cond);
            sites.visit_block(&expr.then_branch);
        });
        if let Some((_, otherwise)) = &expr.else_branch {
            self.visit_expr(otherwise);
        }
    }

    fn visit_expr_while(&mut self, expr: &'ast syn::ExprWhile) {
        self.with_locals(|sites| syn::visit::visit_expr_while(sites, expr));
    }

    fn visit_expr_for_loop(&mut self, expr: &'ast syn::ExprForLoop) {
        self.visit_expr(&expr.expr);
        let state = self.value_state(&expr.expr);
        self.with_locals(|sites| {
            sites.bind(&expr.pat, state);
            sites.visit_pat(&expr.pat);
            sites.visit_block(&expr.body);
        });
    }

    fn visit_expr_match(&mut self, expr: &'ast syn::ExprMatch) {
        self.visit_expr(&expr.expr);
        let state = self.value_state(&expr.expr);
        for arm in &expr.arms {
            self.with_locals(|sites| {
                sites.bind(&arm.pat, state);
                sites.visit_arm(arm);
            });
        }
    }

    fn visit_expr_closure(&mut self, closure: &'ast syn::ExprClosure) {
        // A typed parameter is what its type says; an untyped one is a
        // spawner when the closure is handed to a spawner's method, and not
        // known otherwise.
        let handed = std::mem::take(&mut self.spawner_closure);
        let untyped = match handed {
            true => State::Spawner,
            false => State::Unknown,
        };
        self.with_locals(|sites| {
            for input in &closure.inputs {
                sites.bind(input, untyped);
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
        // `thread_local!` holding a spawner: a static of a spawner type.
        let thread_local = mac
            .path
            .segments
            .last()
            .is_some_and(|last| last.ident == "thread_local");
        if thread_local && token_words(mac.tokens.clone()).iter().any(|w| is_type(w)) {
            self.record("`thread_local!` of a spawner type".to_string());
        }
        self.tokens(mac.tokens.clone());
        syn::visit::visit_macro(self, mac);
    }
}

/// Every spawn site of `files`: (file, site, what).
pub(super) fn reader_sites(files: &[(String, syn::File)]) -> Vec<(String, String, String)> {
    let mut found = vec![];
    for (path, parsed) in files {
        let mut sites = Sites {
            file: path,
            scope: vec![],
            locals: vec![BTreeMap::new()],
            ever: vec![BTreeSet::new()],
            generics: vec![BTreeSet::new()],
            spawner_closure: false,
            found: vec![],
        };
        sites.visit_file(parsed);
        found.extend(sites.found);
    }
    found
}
