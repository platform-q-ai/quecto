//! ADR-0029 rule 5 (#2513 review): a tool call's own work leaves the async
//! workers only through `call_work`, in the call's panic scope, so a panic
//! in it fails the call instead of aborting the process. A test-only guard
//! against accidents, checked over every function of every production file
//! under `src/infrastructure/tools` and `src/interface/tools` (free
//! functions, impls, trait bodies, nested functions, module scope and macro
//! tokens alike). Outside the allowlisted sites, each with its reason, it
//! finds:
//!
//! - a spawn word named anywhere (the reader's `SPAWN_WORDS`:
//!   `spawn_blocking`, `spawn_local`, `spawn_on`, `block_on`,
//!   `block_in_place`, ...);
//! - `spawn`, `scope` or `enter` after a spawner in a path
//!   (`tokio::spawn`, `std::thread::scope`, `<Handle>::spawn`,
//!   `JoinSet::<()>::spawn`, `rayon::spawn`), in a macro too;
//! - `.spawn`, `.scope` or `.enter` on a receiver that names a spawner
//!   (`Handle::current().spawn(..)`, `self.handle.spawn(..)`) or is a local
//!   bound to one: by `let` (from its value or its type), `if let`,
//!   `while let`, a `match` arm, a parameter's type, a closure passed to a
//!   spawner's method, or a copy of another such local. The receiver's own
//!   path is read, never its call arguments (`Command::new(runtime)` is no
//!   spawner), and a local bound to anything else is no spawner whatever
//!   its name;
//! - a `use` of a spawn form, and any `use` or `extern crate` that renames
//!   or globs a spawner or a spawning module, or a `type` alias of a
//!   spawner: a name that could hide a spawn is itself a site.
//!
//! Every entry must still be needed. What it cannot see, syntax alone not
//! following calls or types: a spawn inside a helper outside the checked
//! trees (elsewhere in the crate, or in a dependency); a spawner reached
//! through a value whose origin it does not see (a field or a return value
//! whose path names no spawner, an untyped closure parameter of a plain
//! function); and a local bound inside a macro (a macro's `.spawn` is
//! read by its receiver's words, and the locals bound outside it).

use std::collections::BTreeSet;
use std::path::Path;

#[path = "tool_work_spawns_visitor.rs"]
mod visitor;
use visitor::spawn_sites;

/// The checked trees.
const TREES: &[&str] = &["src/infrastructure/tools", "src/interface/tools"];

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
        // The probes of #2513's round-3 verify.
        (
            "fn f() { if let Ok(h) = tokio::runtime::Handle::try_current() { h.spawn(async {}); } }",
            true,
        ),
        (
            "fn f() { while let Some(h) = Handle::try_current().ok() { h.spawn(async {}); } }",
            true,
        ),
        (
            "fn f() { match Handle::try_current() { Ok(h) => { h.spawn(async {}); } Err(_) => {} } }",
            true,
        ),
        (
            "fn f() { Handle::try_current().map(|h| h.spawn(async {})); }",
            true,
        ),
        ("fn f() { let g = |h: Handle| h.spawn(async {}); }", true),
        ("use tokio::runtime::Runtime as R; fn f() {}", true),
        ("use tokio::task::JoinSet as Set; fn f() {}", true),
        ("use std::thread::Builder as B; fn f() {}", true),
        (
            "use tokio::runtime::{Builder as B, Handle}; fn f() {}",
            true,
        ),
        ("type H = tokio::runtime::Handle; fn f() {}", true),
        ("fn f() { type S = JoinSet<()>; }", true),
        ("extern crate tokio as tk; fn f() {}", true),
        ("extern crate rayon as r; fn f() {}", true),
        (
            "fn f(h: Handle) { let h2 = h.clone(); h2.spawn(async {}); }",
            true,
        ),
        (
            "impl X { fn f(&self) { let h = self.handle.clone(); h.spawn(async {}); } }",
            true,
        ),
        (
            "impl X { fn f(&self) { let r = self.rt.clone(); let s = r; s.spawn(async {}); } fn g(rt: Runtime) { let a = rt; a.spawn(async {}); } }",
            true,
        ),
        (
            "fn f(set: &mut JoinSet<()>) { set.spawn_on(async {}, &h); }",
            true,
        ),
        (
            "fn f() { tokio::task::spawn_blocking_on(|| (), &h); }",
            true,
        ),
        (
            "fn f() { tokio::task::spawn_local_on(async {}, &h); }",
            true,
        ),
        ("fn f() { <Handle>::spawn(&h, async {}); }", true),
        (
            "fn f() { <tokio::runtime::Handle>::current(); <Handle as Clone>::clone(&h).spawn(async {}); }",
            true,
        ),
        (
            "fn f() { let mut s: JoinSet<()> = Default::default(); s.spawn(async {}); }",
            true,
        ),
        (
            "fn f() { drop(format!(\"{:?}\", tokio::task::JoinSet::<()>::spawn)); }",
            true,
        ),
        (
            "fn f() { drop(format!(\"{:?}\", JoinSet::<Vec<()>>::new().spawn(async {}))); }",
            true,
        ),
        ("fn f(h: &Handle) { let _g = h.enter(); }", true),
        (
            "fn f() { let _g = tokio::runtime::Handle::current().enter(); }",
            true,
        ),
        // Not spawns: a process, a scoped name elsewhere, a carried spawn.
        (
            "fn f(runtime: &str) { std::process::Command::new(runtime).spawn().unwrap(); }",
            false,
        ),
        (
            "fn f() { let mut builder = std::process::Command::new(\"x\"); builder.spawn().unwrap(); }",
            false,
        ),
        (
            "fn f(builder: &mut std::process::Command) { builder.spawn().unwrap(); }",
            false,
        ),
        (
            "fn f() { let handle = std::process::Command::new(\"x\").spawn().unwrap(); handle.wait(); }",
            false,
        ),
        ("use std::process::Command as C; fn f() {}", false),
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
