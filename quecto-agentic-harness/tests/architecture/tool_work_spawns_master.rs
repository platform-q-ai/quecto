//! The spawn check as #2513 merged it (`d8c77440c`), kept as it was: the
//! baseline the reader must cover (`the_reader_catches_all_the_merged_check_caught`).
//! Every form this catches, the reader catches, but the false positives
//! it was written to leave (a `Command` named `runtime` or `builder`, and
//! `Client::builder()`).

use std::collections::BTreeSet;

use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use syn::visit::Visit;

/// Words that spawn, block on or enter a runtime wherever they are named.
const SPAWN_WORDS: &[&str] = &[
    "spawn_blocking",
    "spawn_local",
    "spawn_ok",
    "block_on",
    "block_in_place",
];

/// The path segments before `spawn` (or `scope`) that make it a task or
/// thread spawn (`tokio::spawn`, `tokio::task::spawn`, `std::thread::spawn`,
/// `std::thread::scope`, `Handle::spawn`, `JoinSet::spawn`,
/// `rayon::spawn`), as a process's `Command::spawn` is not.
const SPAWNERS: &[&str] = &[
    "tokio", "task", "thread", "Handle", "Runtime", "JoinSet", "rayon",
];

/// The words after a spawner that spawn: `scope` runs scoped threads.
const SPAWNED_BY_SPAWNER: &[&str] = &["spawn", "scope"];

/// What a `.spawn(..)` receiver names when it spawns a task or a thread: a
/// runtime, its handle, a join set, or a thread builder or scope.
const SPAWN_RECEIVERS: &[&str] = &[
    "Handle", "handle", "runtime", "Runtime", "Builder", "builder", "thread", "JoinSet",
];

/// A `use` that renames or globs one of these hides the spawns after it.
const SPAWNING_MODULES: &[&[&str]] = &[
    &["tokio"],
    &["tokio", "task"],
    &["tokio", "runtime"],
    &["std", "thread"],
];

/// What a local's value names when the local then spawns (`let rt =
/// Runtime::new()?; rt.spawn(..)`): a runtime, its handle, a join set, a
/// builder or a thread pool.
const SPAWNER_SOURCES: &[&str] = &["Handle", "Runtime", "JoinSet", "Builder", "ThreadPool"];

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
        _ => {}
    }
}

struct Sites<'a> {
    file: &'a str,
    scope: Vec<String>,
    /// Per scope, the locals whose value names a spawner.
    spawner_locals: Vec<BTreeSet<String>>,
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
        self.spawner_locals.push(BTreeSet::new());
        visit(self);
        self.spawner_locals.pop();
        self.scope.pop();
    }

    /// Whether `word` is a local, in this scope or one around it, whose
    /// value names a spawner.
    fn spawner_local(&self, word: &str) -> bool {
        self.spawner_locals
            .iter()
            .any(|locals| locals.contains(word))
    }

    /// Whether a `.spawn(..)` receiver, as words, spawns a task or thread.
    fn spawning_receiver(&self, words: &[String]) -> bool {
        words
            .iter()
            .any(|word| SPAWN_RECEIVERS.contains(&word.as_str()) || self.spawner_local(word))
    }

    /// A path that names a spawn form: a spawn word anywhere, or `spawn`
    /// after a spawner.
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

    /// A macro's tokens held to the same rules: a spawn word, `spawn` or
    /// `scope` after a spawner and `::`, or `.spawn` on a spawning
    /// receiver (back to the start of its expression). A group stands in
    /// `seen` as one marker, so a lookback never joins tokens across it.
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
                    let by_path = SPAWNED_BY_SPAWNER.contains(&word.as_str())
                        && seen.ends_with(&[":".to_string(), ":".to_string()])
                        && seen
                            .iter()
                            .rev()
                            .nth(2)
                            .is_some_and(|before| SPAWNERS.contains(&before.as_str()));
                    let by_method = word == "spawn"
                        && seen.last().is_some_and(|before| before == ".")
                        && self.spawning_receiver(
                            &seen
                                .iter()
                                .rev()
                                .take_while(|token| {
                                    !matches!(token.as_str(), ";" | "," | "=" | "=>")
                                })
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

/// A `use` leaf: its path, and whether it renames or globs a spawning
/// module or type ([`SPAWNING_MODULES`], `Handle`).
type UseLeaf = (Vec<String>, bool);

/// Whether `path` is a spawning module or ends in `Handle`.
fn spawning_import(path: &[String]) -> bool {
    SPAWNING_MODULES
        .iter()
        .any(|module| module.iter().copied().eq(path.iter().map(String::as_str)))
        || path.last().is_some_and(|last| last == "Handle")
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
            let hides = spawning_import(&path);
            out.push((path, hides));
        }
        syn::UseTree::Group(group) => {
            for tree in &group.items {
                use_leaves(tree, prefix, out);
            }
        }
        syn::UseTree::Glob(_) => out.push((prefix.clone(), spawning_import(prefix))),
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
                syn::visit::visit_impl_item_fn(sites, item)
            });
        }
    }

    fn visit_trait_item_fn(&mut self, item: &'ast syn::TraitItemFn) {
        self.scoped(item.sig.ident.to_string(), |sites| {
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

    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.path(&words(path));
        syn::visit::visit_path(self, path);
    }

    fn visit_local(&mut self, local: &'ast syn::Local) {
        let names_spawner = local.init.as_ref().is_some_and(|init| {
            token_words(init.expr.to_token_stream())
                .iter()
                .any(|word| SPAWNER_SOURCES.contains(&word.as_str()))
        });
        if names_spawner {
            let mut names = vec![];
            bound_names(&local.pat, &mut names);
            if let Some(locals) = self.spawner_locals.last_mut() {
                locals.extend(names);
            }
        }
        syn::visit::visit_local(self, local);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let method = call.method.to_string();
        let receiver = call.receiver.to_token_stream().to_string();
        let spawns = method == "spawn"
            && self.spawning_receiver(&token_words(call.receiver.to_token_stream()));
        if SPAWN_WORDS.contains(&method.as_str()) || spawns {
            self.record(format!("`.{method}(..)` on `{receiver}`"));
        }
        syn::visit::visit_expr_method_call(self, call);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        self.tokens(mac.tokens.clone());
        syn::visit::visit_macro(self, mac);
    }
}

/// Every spawn site of `files`: (file, site, what).
pub(super) fn master_sites(files: &[(String, syn::File)]) -> Vec<(String, String, String)> {
    let mut found = vec![];
    for (path, parsed) in files {
        let mut sites = Sites {
            file: path,
            scope: vec![],
            spawner_locals: vec![BTreeSet::new()],
            found: vec![],
        };
        sites.visit_file(parsed);
        found.extend(sites.found);
    }
    found
}
