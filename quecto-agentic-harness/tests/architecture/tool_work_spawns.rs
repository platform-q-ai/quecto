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
