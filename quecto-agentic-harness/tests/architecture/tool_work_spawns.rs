//! ADR-0029 rule 5 (#2513 review): a tool call's own work leaves the async
//! workers only through `call_work`, in the call's panic scope, so a panic
//! in it fails the call instead of aborting the process. A test-only guard
//! against accidents, checked over every function of every production file
//! under `src/infrastructure/tools` and `src/interface/tools` (free
//! functions, impls, trait bodies, nested functions, module scope and macro
//! tokens alike). Outside the allowlisted sites, each with its reason, no
//! site may be flagged; every entry must still be needed.
//!
//! The check is the union, site by site (file, function and what), of two
//! readers: whatever either flags is flagged
//! (`the_tree_is_held_to_both_checks`,
//! `every_site_either_check_finds_is_flagged`).
//!
//! - **The check as merged with #2513** (`tool_work_spawns_master.rs`,
//!   unchanged): a spawn word anywhere (`spawn_blocking`, `spawn_local`,
//!   `spawn_ok`, `block_on`, `block_in_place`); `spawn` or `scope` after a
//!   spawner in a path (`tokio::spawn`, `std::thread::scope`,
//!   `JoinSet::spawn`, `rayon::spawn`); `.spawn` on a receiver any of whose
//!   words is a spawner type or name (`Handle`, `handle`, `Runtime`,
//!   `runtime`, `Builder`, `builder`, `thread`, `JoinSet`) or a local whose
//!   value named one; a `use` of a spawn form, or one that renames or globs
//!   `tokio`, `tokio::task`, `tokio::runtime`, `std::thread` or `Handle`;
//!   and the same rules over macro tokens.
//! - **The reader** (`tool_work_spawns_visitor.rs` and
//!   `tool_work_spawns_values.rs`), for what the merged check misses: more
//!   spawn words (`spawn_on`, `spawn_scoped`, `spawn_unchecked`,
//!   `spawn_with_handle`, `spawn_obj`, `spawn_fifo`, `spawn_pinned`,
//!   `scope_fifo`, `in_place_scope`, `run_until`, `par_iter`,
//!   `block_on_stream`, ...); more spawners in paths (`smol`, `async_std`,
//!   `crossbeam`, `actix_rt`, `async_global_executor`, `TaskTracker`) and
//!   `enter` after one; `.scope` and `.enter` on a spawning receiver;
//!   qualified self (`<Handle>::spawn`) and a turbofish in a macro; raw
//!   identifiers (`r#spawn`); a value that holds a spawner through a
//!   wrapper, a tail, a cast or a turbofish (`Arc::new(Handle::current())`,
//!   `{ h }`, `x as Handle`, `collect::<JoinSet<_>>()`), `.runtime()` or
//!   `handle()`; locals bound to a spawner by `let`, assignment, `if let`,
//!   `while let`, `match` arms, `for` loops, typed parameters, closures
//!   handed to a spawner's method, struct-pattern fields and `@`
//!   subpatterns, each block, closure and arm its own scope; a rename of any
//!   path ending in a spawner, an `extern crate` rename, and a `type`
//!   alias, `static`, `const` (associated ones included), `thread_local!`
//!   or generic default of a spawner type; and `#[tokio::main]`.
//!
//! What both miss, syntax alone not following calls or types: a spawn
//! inside a helper outside the checked trees (elsewhere in the crate, or
//! in a dependency); a spawner reached through a value none of whose words
//! names one (`self.pool.get().spawn(..)`, `for h in self.handles.iter()`),
//! or through a generic executor (`E::spawn(..)`); rayon's `join` and
//! `install`; an identifier a macro splices together (`paste!`), code a
//! macro pulls in (`include!`), and a local bound inside a macro.

use std::collections::BTreeSet;
use std::path::Path;

#[path = "tool_work_spawns_values.rs"]
mod values;
#[path = "tool_work_spawns_visitor.rs"]
mod visitor;
use visitor::reader_sites;
#[path = "tool_work_spawns_cases.rs"]
mod cases;
#[path = "tool_work_spawns_master.rs"]
mod master;

/// Every spawn site of `files`: the union, site by site (file, function
/// and what), of the check as merged with #2513 and the reader: whatever
/// either flags is flagged.
fn spawn_sites(files: &[(String, syn::File)]) -> Vec<(String, String, String)> {
    let mut sites = reader_sites(files);
    for site in master::master_sites(files) {
        if !sites.contains(&site) {
            sites.push(site);
        }
    }
    sites
}

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
        "src/infrastructure/tools/coordinator_launch/mod.rs",
        "impl SpawnCoordinatorLauncher::launch",
        "drives the session's spawn tool to completion in place, on the board call's own blocking thread (no new task): its panic is the board call's",
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

/// On the real tree, the sites flagged are exactly the merged check's and
/// the reader's, site by site.
#[test]
fn the_tree_is_held_to_both_checks() {
    let files = checked_files();
    let flagged: BTreeSet<(String, String, String)> = spawn_sites(&files).into_iter().collect();
    let both: BTreeSet<(String, String, String)> = master::master_sites(&files)
        .into_iter()
        .chain(reader_sites(&files))
        .collect();
    assert_eq!(flagged, both);
    assert!(!flagged.is_empty(), "the tree has its allowlisted sites");
}
