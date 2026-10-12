//! ADR-0029 rule 5 (#2513 review): a tool call's own work leaves the async
//! workers only through `call_work`, in the call's panic scope, so a panic
//! in it fails the call instead of aborting the process. Proven over every
//! function of every production file under `src/infrastructure/tools` and
//! `src/interface/tools` (free functions, impls, trait bodies, nested
//! functions, module scope and macro tokens alike): nothing spawns a task,
//! a blocking job or a thread, or blocks on or enters a runtime, but the
//! allowlisted sites, each with its reason. A `use` of a spawn form is
//! itself a site, so an import cannot hide one. Every entry must still be
//! needed.

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
    "block_on",
    "block_in_place",
];

/// The path segments before `spawn` that make it a task or thread spawn
/// (`tokio::spawn`, `tokio::task::spawn`, `std::thread::spawn`,
/// `Handle::spawn`), as a process's `Command::spawn` is not.
const SPAWNERS: &[&str] = &["tokio", "task", "thread", "Handle", "Runtime"];

/// What a `.spawn(..)` receiver names when it spawns a task or a thread: a
/// runtime handle or a thread builder.
const SPAWN_RECEIVERS: &[&str] = &[
    "Handle", "handle", "runtime", "Runtime", "Builder", "thread",
];

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
        "src/infrastructure/tools/filesystem/fs_failure.rs",
        "fn run_write",
        "pre-existing (#2254): a panicked write is read as a fault whose leftover is unknown; moving it onto call_work changes that answer (#2517)",
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
        "watches a container child for its lifetime, which outlives the spawn call",
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
        "tears sub-agents down at exit on its own executor, when no runtime runs",
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
        "the swarm watcher's own thread and runtime (#2278), never an async worker",
    ),
    (
        "src/infrastructure/tools/swarm_lifecycle.rs",
        "impl Supervisor<'_>::announce",
        "on the swarm watcher's own runtime (#2278)",
    ),
    (
        "src/infrastructure/tools/swarm_lifecycle.rs",
        "fn watch_until_ended",
        "on the swarm watcher's own runtime (#2278)",
    ),
    (
        "src/infrastructure/tools/swarm_lifecycle.rs",
        "fn settle_observed_snapshot",
        "on the swarm watcher's own runtime (#2278)",
    ),
];

struct Sites<'a> {
    file: &'a str,
    scope: Vec<String>,
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
        visit(self);
        self.scope.pop();
    }

    /// A path that names a spawn form: a spawn word anywhere, or `spawn`
    /// after a spawner.
    fn path(&mut self, words: &[String]) {
        let spawn_word = words
            .iter()
            .any(|word| SPAWN_WORDS.contains(&word.as_str()));
        let spawn_after_spawner = words.iter().enumerate().any(|(index, word)| {
            word == "spawn"
                && index > 0
                && words[..index]
                    .iter()
                    .any(|before| SPAWNERS.contains(&before.as_str()))
        });
        if spawn_word || spawn_after_spawner {
            self.record(format!("`{}`", words.join("::")));
        }
    }

    /// A macro's tokens held to the same rules: a spawn word, or `spawn`
    /// after a spawner and `::`.
    fn tokens(&mut self, stream: TokenStream) {
        let mut seen: Vec<String> = vec![];
        for tree in stream {
            match &tree {
                TokenTree::Group(group) => self.tokens(group.stream()),
                TokenTree::Ident(ident) => {
                    let word = ident.to_string();
                    let after_spawner = word == "spawn"
                        && seen
                            .iter()
                            .rev()
                            .take(3)
                            .any(|before| SPAWNERS.contains(&before.as_str()));
                    if SPAWN_WORDS.contains(&word.as_str()) || after_spawner {
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

/// The leaves of a `use` tree, each with the path before it.
fn use_leaves(tree: &syn::UseTree, prefix: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            use_leaves(&path.tree, prefix, out);
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            out.push([prefix.clone(), vec![name.ident.to_string()]].concat())
        }
        syn::UseTree::Rename(rename) => {
            out.push([prefix.clone(), vec![rename.ident.to_string()]].concat())
        }
        syn::UseTree::Group(group) => {
            for tree in &group.items {
                use_leaves(tree, prefix, out);
            }
        }
        syn::UseTree::Glob(_) => {}
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
        for leaf in leaves {
            self.path(&leaf);
        }
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        self.path(&words(path));
        syn::visit::visit_path(self, path);
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        let method = call.method.to_string();
        let receiver = call.receiver.to_token_stream().to_string();
        let spawns =
            method == "spawn" && SPAWN_RECEIVERS.iter().any(|name| receiver.contains(name));
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
        (
            "fn f(c: &mut std::process::Command) { c.spawn().unwrap(); }",
            false,
        ),
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
