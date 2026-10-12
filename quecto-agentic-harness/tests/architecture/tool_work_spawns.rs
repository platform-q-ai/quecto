//! ADR-0029 rule 5 (#2513 review): a tool call's own work leaves the async
//! workers only through `call_work`, in the call's panic scope, so a panic
//! in it fails the call instead of aborting the process. Proven over every
//! function of every production file under `src/infrastructure/tools` and
//! `src/interface/tools` (free functions, impls, trait bodies, nested
//! functions, module scope and macro tokens alike): nothing spawns a task,
//! a blocking job or a thread, or blocks on or enters a runtime, but the
//! allowlisted sites, each with its reason. A `use` of a spawn form is
//! itself a site, and so is a `use` that renames or globs a spawning module
//! or type (`tokio`, `tokio::task`, `tokio::runtime`, `std::thread`,
//! `Handle`), so an import cannot hide one. Every entry must still be
//! needed.
//!
//! What syntax alone cannot see: a spawn inside a helper outside the
//! checked trees (elsewhere in the crate, or in a dependency) that a tool
//! calls by another name, and a spawner reached through a value whose
//! origin is not named in the same function (a field, a parameter, a
//! return value) unless its receiver's text names a runtime, handle,
//! builder or thread. The checked trees' own helpers are seen, since they
//! are checked too.

use std::collections::BTreeSet;
use std::path::Path;

use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use syn::visit::Visit;

/// The checked trees.
const TREES: &[&str] = &["src/infrastructure/tools", "src/interface/tools"];

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

/// The sites that may spawn, by file and site, and why: the carrier itself,
/// work that outlives a call by design, and work on threads and runtimes of
/// its own.
const ALLOWED: &[(&str, &str, &str)] = &[
    (
        "src/infrastructure/tools/call_work.rs",
        "fn spawn_blocking_in_call",
        "the one place a call carries blocking work: joined through join_carried",
    ),
    (
        "src/infrastructure/tools/call_work.rs",
        "fn spawn_blocking_in_call_on",
        "carries blocking work onto a given runtime: joined through join_carried",
    ),
    (
        "src/infrastructure/tools/call_work.rs",
        "fn spawn_in_call",
        "the one place a call carries a task: joined through join_carried",
    ),
    (
        "src/infrastructure/tools/call_work.rs",
        "fn block_here",
        "runs a call's blocking work in place on a multi-thread worker",
    ),
    (
        "src/infrastructure/tools/call_work.rs",
        "fn off_the_runtime",
        "test support only: a test's blocking setup on a scoped thread, joined, its panic resumed",
    ),
    (
        "src/infrastructure/tools/call_work.rs",
        "fn std_thread_in_call",
        "the call's own detached thread, carried into its scope: a panic fails the call through execute_contained's backstop",
    ),
    (
        "src/infrastructure/tools/find_fd.rs",
        "impl FdFindPaths::find",
        "drives its own current-thread runtime on the find owner's thread (call_work::std_thread_in_call), never a worker",
    ),
    (
        "src/infrastructure/tools/panic_probe.rs",
        "impl PanicProbeTool::execute",
        "test-support fault injection (#2192): a panic outside any call, on purpose",
    ),
    (
        "src/infrastructure/tools/spawn_container.rs",
        "fn spawn_script_managed_child",
        "runs the container create as its own task, tracked by InFlight: a create the dropped spawn call no longer claims is rolled back",
    ),
    (
        "src/infrastructure/tools/spawn_launch_guard.rs",
        "impl SpawnLaunchPorts<'_>::spawn_compensation",
        "rolls a failed launch back after the call, tracked by InFlight",
    ),
    (
        "src/infrastructure/tools/spawn_launch_guard.rs",
        "impl SpawnLaunchPorts<'_>::abandon_unregistered",
        "retires an unregistered child's admission scope after the call, tracked by InFlight",
    ),
    (
        "src/infrastructure/tools/spawn_prepared.rs",
        "impl PreparedChild::drop",
        "rolls back a launch dropped unrun, tracked by InFlight",
    ),
    (
        "src/infrastructure/tools/spawn_proxy_bridge.rs",
        "fn materialize",
        "the proxy bridge's accept loop: serves a child for its lifetime",
    ),
    (
        "src/infrastructure/tools/spawn_proxy_bridge.rs",
        "fn accept_loop",
        "the proxy bridge's per-connection bridges: serve a child for its lifetime",
    ),
    (
        "src/infrastructure/tools/spawn_reaper.rs",
        "fn spawn_reaper_task",
        "reaps spawned children for the process's life; its blocking wait is call_work::blocking",
    ),
    (
        "src/infrastructure/tools/subagent_cleanup.rs",
        "fn run_cleanup_jobs_sync",
        "tears sub-agents down on its own executor: at exit, and from inside spawn_blocking while the runtime runs (run_cleanup_jobs)",
    ),
    (
        "src/infrastructure/tools/subagent_cleanup.rs",
        "fn run_cleanup_jobs",
        "tears sub-agents down off the workers (call_work::blocking)",
    ),
    (
        "src/infrastructure/tools/subagent_monitor.rs",
        "fn spawn_monitor_task",
        "watches sub-agents for the session",
    ),
    (
        "src/infrastructure/tools/subagent_monitor/swarm_wake.rs",
        "fn arm",
        "a wake timer for the parent, outliving the call",
    ),
    (
        "src/infrastructure/tools/subagent_monitor/worker_wake.rs",
        "fn worker_turn_end",
        "a worker's wake for the parent, outliving the call",
    ),
    (
        "src/infrastructure/tools/subagent_monitor_stall.rs",
        "fn retain_pending_stall",
        "a stall timer for the parent, outliving the call",
    ),
    (
        "src/infrastructure/tools/swarm_coordination.rs",
        "impl SwarmContext::record",
        "records swarm coordination off the workers (call_work::blocking)",
    ),
    (
        "src/infrastructure/tools/swarm_lifecycle.rs",
        "fn supervise",
        "the swarm watcher's own thread, which builds a fresh current-thread runtime (#2278), never an async worker",
    ),
    (
        "src/infrastructure/tools/swarm_lifecycle.rs",
        "impl Supervisor<'_>::announce",
        "builds a fresh current-thread runtime on the swarm watcher's thread (#2278)",
    ),
    (
        "src/infrastructure/tools/swarm_lifecycle.rs",
        "fn watch_until_ended",
        "blocks on the fresh current-thread runtime supervise built, on the swarm watcher's thread (#2278)",
    ),
    (
        "src/infrastructure/tools/swarm_lifecycle.rs",
        "fn settle_observed_snapshot",
        "builds a fresh current-thread runtime on the swarm watcher's thread (#2278)",
    ),
];

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

/// The production files of the checked trees, parsed.
fn checked_files() -> Vec<(String, syn::File)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = vec![];
    let mut stack: Vec<_> = TREES.iter().map(|tree| root.join(tree)).collect();
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            match (
                path.is_dir(),
                name.ends_with(".rs") && !name.ends_with("_tests.rs"),
            ) {
                (true, _) => stack.push(path),
                (false, true) => {
                    let source = std::fs::read_to_string(&path).unwrap();
                    let relative = path.strip_prefix(root).unwrap().display().to_string();
                    let parsed =
                        syn::parse_file(&source).unwrap_or_else(|e| panic!("{relative}: {e}"));
                    files.push((relative, parsed));
                }
                (false, false) => {}
            }
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

/// Every spawn site of `files`: (file, site, what).
fn spawn_sites(files: &[(String, syn::File)]) -> Vec<(String, String, String)> {
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

/// Every spawn site of `files` outside [`ALLOWED`], and every stale entry.
fn violations(files: &[(String, syn::File)]) -> Vec<String> {
    let found = spawn_sites(files);
    let mut used = BTreeSet::new();
    let mut out = vec![];
    for (file, site, what) in &found {
        match ALLOWED.iter().position(|(allowed, at, _)| allowed == file && at == site) {
            Some(entry) => {
                used.insert(entry);
            }
            None => out.push(format!("{file}: `{site}` spawns {what}: go through call_work, or allowlist it with its reason")),
        }
    }
    for (index, (file, site, _)) in ALLOWED.iter().enumerate() {
        if !used.contains(&index) {
            out.push(format!(
                "ALLOWED lists {file}: `{site}`, which spawns nothing: remove it"
            ));
        }
    }
    out
}

#[test]
fn tool_work_is_spawned_only_through_call_work() {
    let files = checked_files();
    assert!(files.len() > 50, "the tool trees are read: {}", files.len());
    let violations = violations(&files);
    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

/// The checker sees every spawn form, however it is reached.
#[test]
fn every_spawn_form_is_seen() {
    for (source, seen) in [
        ("fn f() { tokio::task::spawn_blocking(|| ()); }", true),
        (
            "use tokio::task::spawn_blocking; fn f() { spawn_blocking(|| ()); }",
            true,
        ),
        (
            "use tokio::task::{self as t}; fn f() { t::spawn_blocking(|| ()); }",
            true,
        ),
        ("fn f() { std::thread::spawn(|| ()); }", true),
        ("use std::thread; fn f() { thread::spawn(|| ()); }", true),
        (
            "fn f() { tokio::runtime::Handle::current().spawn(async {}); }",
            true,
        ),
        (
            "fn f(h: tokio::runtime::Handle) { h.block_on(async {}); }",
            true,
        ),
        ("fn f() { tokio::task::spawn_local(async {}); }", true),
        (
            "fn f() { std::thread::Builder::new().spawn(|| ()).unwrap(); }",
            true,
        ),
        ("fn f() { tokio::spawn(async {}); }", true),
        (
            "fn f() { let _ = format!(\"{:?}\", tokio::spawn(async {})); }",
            true,
        ),
        (
            "impl X { fn f(&self) { self.handle.spawn(async {}); } }",
            true,
        ),
        // The bypasses #2513's third review listed.
        ("fn f() { tokio::task::JoinSet::<()>::spawn; }", true),
        (
            "fn f() { let mut set = tokio::task::JoinSet::new(); set.spawn(async {}); }",
            true,
        ),
        (
            "use tokio::task::JoinSet; fn f() { let mut s = JoinSet::new(); s.spawn(async {}); }",
            true,
        ),
        (
            "fn f() { std::thread::scope(|s| { s.spawn(|| ()); }); }",
            true,
        ),
        ("use std::thread; fn f() { thread::scope(|s| ()); }", true),
        ("fn f(pool: &ThreadPool) { pool.spawn_ok(async {}); }", true),
        ("fn f() { rayon::spawn(|| ()); }", true),
        (
            "fn f(builder: std::thread::Builder) { builder.spawn(|| ()).unwrap(); }",
            true,
        ),
        (
            "fn f() { let h = tokio::runtime::Handle::current(); h.spawn(async {}); }",
            true,
        ),
        (
            "fn f() { let rt = tokio::runtime::Runtime::new().unwrap(); rt.spawn(async {}); }",
            true,
        ),
        ("use tokio as tk; fn f() { tk::spawn(async {}); }", true),
        ("use tokio::task as t; fn f() { t::spawn(async {}); }", true),
        ("use tokio::runtime as r; fn f() {}", true),
        ("use std::thread as th; fn f() { th::spawn(|| ()); }", true),
        ("use tokio::*; fn f() { spawn(async {}); }", true),
        ("use tokio::task::*; fn f() { spawn(async {}); }", true),
        ("use std::thread::*; fn f() { spawn(|| ()); }", true),
        (
            "use tokio::runtime::Handle as H; fn f() { H::current().spawn(async {}); }",
            true,
        ),
        (
            "fn f() { drop(format!(\"{:?}\", tokio::runtime::Handle::current().spawn(async {}))); }",
            true,
        ),
        (
            "fn f() { let rt = Runtime::new().unwrap(); drop(format!(\"{:?}\", rt.spawn(async {}))); }",
            true,
        ),
        // Not spawns: a process, a scoped name elsewhere, a carried spawn.
        (
            "fn f(c: &mut std::process::Command) { c.spawn().unwrap(); }",
            false,
        ),
        (
            "fn f() { let mut c = std::process::Command::new(\"x\"); c.spawn().unwrap(); }",
            false,
        ),
        ("use std::thread::sleep; fn f() {}", false),
        (
            "fn f() { call_work::spawn_blocking_in_call(|| ()); }",
            false,
        ),
        (
            "#[cfg(test)] mod tests { fn f() { tokio::spawn(async {}); } }",
            false,
        ),
    ] {
        let file = syn::parse_file(source).unwrap();
        let sites = spawn_sites(&[("x.rs".to_string(), file)]);
        assert_eq!(!sites.is_empty(), seen, "{source}: {sites:?}");
    }
}
