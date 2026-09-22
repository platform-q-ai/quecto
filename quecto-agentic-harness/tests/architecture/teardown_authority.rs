//! Epic #1929 close (#1940): the affirmative process-effect allowlist, the
//! "no authority from a pid" ratchets, the retired-name sweep and the
//! whole-crate layer baselines. Every list here is exact: a file that stops
//! matching fails the test as loudly as a new violation, so the allowlist
//! can never rot into a denylist.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

/// Production lines of a source file, numbered: every `#[cfg(test)]`-gated
/// ITEM is removed — the attribute, any attributes that follow it, and the
/// one item they gate (a `;`-terminated `use`/`mod x;`, or a braced item
/// such as `mod tests { … }` / `fn`, to its matching close brace) — and
/// comment-only lines are dropped so a doc comment naming `kill(2)` is not
/// an effect. Nothing else is cut: an early `#[cfg(test)] use …` no longer
/// hides the rest of the file (review of #1940).
pub(super) fn production_code(path: &str) -> Vec<(usize, String)> {
    stripped(path)
        .iter()
        .filter(|(_, line)| {
            let trimmed = line.trim_start();
            !(trimmed.starts_with("//") || trimmed.is_empty())
        })
        .cloned()
        .collect()
}

/// [`production_code`] joined back into one string (comments kept), for the
/// substring ratchets of `tests/architecture.rs`.
pub(super) fn production_source(path: &str) -> String {
    stripped(path)
        .iter()
        .map(|(_, line)| line.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

type StrippedLines = std::sync::Arc<Vec<(usize, String)>>;

/// [`strip_test_items`] of the file at `path`, computed once per process:
/// the ratchets below call `production_code` once per needle per file, and
/// the sources do not change while the binary runs.
fn stripped(path: &str) -> StrippedLines {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, StrippedLines>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some(hit) = cache.lock().unwrap().get(path) {
        return hit.clone();
    }
    let source = fs::read_to_string(path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    let lines = Arc::new(strip_test_items(&source));
    cache
        .lock()
        .unwrap()
        .entry(path.to_string())
        .or_insert(lines)
        .clone()
}

/// Brace delta of one line with string and char literals blanked, so a
/// `"{"` inside a format string does not open an item.
fn brace_delta(line: &str) -> i64 {
    let mut delta = 0i64;
    let mut chars = line.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        match c {
            '\\' if in_string => {
                chars.next();
            }
            '"' => in_string = !in_string,
            '\'' if !in_string => {
                // A char literal (`'{'`) or a lifetime (`'a`): skip a quoted
                // single char, leave lifetimes alone.
                let mut ahead = chars.clone();
                if let (Some(_), Some('\'')) = (ahead.next(), ahead.next()) {
                    chars.next();
                    chars.next();
                }
            }
            '{' if !in_string => delta += 1,
            '}' if !in_string => delta -= 1,
            _ => {}
        }
    }
    delta
}

/// Remove every `#[cfg(test)]`-gated item; keeps the original line numbers.
pub(super) fn strip_test_items(source: &str) -> Vec<(usize, String)> {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim() != "#[cfg(test)]" {
            out.push((i + 1, lines[i].to_string()));
            i += 1;
            continue;
        }
        // Skip the attribute run, then the one item it gates.
        let mut j = i + 1;
        while j < lines.len() && lines[j].trim_start().starts_with("#[") {
            j += 1;
        }
        let mut depth = 0i64;
        let mut opened = false;
        while j < lines.len() {
            let line = lines[j];
            let delta = brace_delta(line);
            if delta != 0 || line.contains('{') {
                opened = true;
            }
            depth += delta;
            j += 1;
            if opened && depth <= 0 {
                break;
            }
            if !opened && line.trim_end().ends_with(';') {
                break;
            }
        }
        i = j;
    }
    out
}

#[test]
fn strip_test_items_removes_only_gated_items() {
    let source = "\
#[cfg(test)]
use x::y;
pub fn keep() {}
#[cfg(test)]
#[path = \"t.rs\"]
mod tests;
fn also_kept() { let s = \"{\"; }
#[cfg(test)]
mod inline {
    fn hidden() { libc::kill(1, 9); }
}
fn last() {}
";
    let kept: Vec<String> = strip_test_items(source)
        .into_iter()
        .map(|(_, line)| line)
        .collect();
    assert_eq!(
        kept,
        vec![
            "pub fn keep() {}",
            "fn also_kept() { let s = \"{\"; }",
            "fn last() {}",
        ]
    );
    let numbers: Vec<usize> = strip_test_items(source)
        .into_iter()
        .map(|(number, _)| number)
        .collect();
    assert_eq!(numbers, vec![3, 7, 12]);
}

/// The scan really reaches the whole file: the files whose first
/// `#[cfg(test)]` gates an early `use` are read to their production end.
#[test]
fn production_code_reads_past_an_early_cfg_test_use() {
    for (path, needle) in [
        ("src/infrastructure/tools/spawn.rs", "pub fn with_base_dir"),
        (
            "src/infrastructure/tools/subagent_registry.rs",
            "pub fn with_identity",
        ),
        (
            "src/infrastructure/tools/subagent_monitor.rs",
            "pub fn spawn_monitor_task",
        ),
        (
            "src/infrastructure/tools/bash/mod.rs",
            "libc::kill(-(pid as libc::pid_t), libc::SIGKILL)",
        ),
        (
            "src/interface/cli/uds_multi.rs",
            "async fn handle_disconnect(",
        ),
    ] {
        assert!(
            production_code(path)
                .iter()
                .any(|(_, line)| line.contains(needle)),
            "{path}: `{needle}` must be visible to the scan"
        );
    }
}

pub(super) fn walk(dir: &Path, out: &mut Vec<String>) {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        .map(|entry| entry.expect("dir entry").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path.to_string_lossy().to_string());
        }
    }
}

/// Every production `.rs` file of the harness crate (test files, `_tests.rs`
/// siblings and `tests/` directories excluded).
pub(super) fn production_files() -> Vec<String> {
    let mut files = Vec::new();
    walk(Path::new("src"), &mut files);
    files
        .into_iter()
        .filter(|path| !path.ends_with("_tests.rs") && !path.contains("/tests/"))
        .collect()
}

fn matches(line: &str, pattern: &str) -> bool {
    match pattern {
        // `\bkill(`: `child.kill()` and `kill(pid, sig)` but not `begin_kill(`
        // or `run_retained_kill(`.
        "kill(" => line.match_indices("kill(").any(|(index, _)| {
            index == 0
                || !line[..index]
                    .chars()
                    .next_back()
                    .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        }),
        _ => line.contains(pattern),
    }
}

/// The vocabulary of a process effect or of process-identity authority.
///
/// Known evasions this textual scan does NOT catch (documented, review of
/// #1940): a raw `libc::syscall(libc::SYS_kill, …)` / `SYS_tgkill` (the
/// constant names are listed below, a numeric syscall id is not),
/// `pthread_kill`/`tgkill` (listed), shelling out (`pkill`, `killall`,
/// `kill` inside a `sh -c` string — only the literal `"pkill"`/`"killall"`
/// and `Command::new("kill")` forms are listed), a helper copied under a
/// different name that wraps one of these (its call site is caught only if
/// the helper's own file is outside the allowlist), and `procfs`-style
/// crates reading `/proc` through a library. The retired-name sweep, the
/// no-pid files and the single-owner list are the other legs of the
/// ratchet; a new signalling mechanism must add its file here on purpose.
const PROCESS_EFFECT_PATTERNS: &[&str] = &[
    "libc::kill",
    "kill(",
    "SYS_kill",
    "SYS_tgkill",
    "tgkill",
    "pthread_kill",
    "\"pkill\"",
    "killall",
    "Command::new(\"kill\")",
    "start_kill",
    "kill_on_drop",
    "killpg",
    "kill_pid",
    "process_group(",
    "setpgid",
    "prctl",
    "libc::SIGKILL",
    "libc::SIGTERM",
    "/proc/",
    "pidfd",
];

/// The ONLY process effects of the harness crate, by file, each with the
/// exact vocabulary it may use. Everything else is a violation.
const PROCESS_EFFECT_ALLOWLIST: &[(&str, &[&str], &str)] = &[
    (
        "src/infrastructure/processes/owned_child_supervisor.rs",
        &["libc::kill", "kill(", "libc::SIGKILL", "libc::SIGTERM"],
        "slice B (#1935): the one owner of a directly launched, unreaped Child; \
         TERM/KILL through its retained handle only",
    ),
    (
        "src/infrastructure/processes/parent_death_signal.rs",
        &["prctl", "libc::SIGTERM"],
        "arms PR_SET_PDEATHSIG on a process this harness spawns (the child is \
         signalled by the kernel when its launcher dies; nothing is sent from here)",
    ),
    (
        "src/infrastructure/tools/bash/mod.rs",
        &[
            "libc::kill",
            "kill(",
            "kill_on_drop",
            "process_group(",
            "libc::SIGKILL",
        ],
        "bash-owned invocation containment of the tool's own process group",
    ),
    (
        "src/infrastructure/tools/swarm_process.rs",
        &[
            "libc::kill",
            "kill(",
            "kill_on_drop",
            "kill_pid",
            "setpgid",
            "libc::SIGKILL",
        ],
        "Python ExecutionScope containment of an execution job this tool spawned",
    ),
    (
        "src/infrastructure/tools/swarm_scope.rs",
        &["kill_pid"],
        "Python ExecutionScope: the job's own group-or-pid containment on scope end",
    ),
    (
        "src/infrastructure/tools/swarm.rs",
        &["kill_pid"],
        "execution-job cancel (`cancel_job_process`, job registry) through the \
         ExecutionScope containment; never a swarm member's harness (#1939)",
    ),
    (
        "src/infrastructure/persistence/session_ownership.rs",
        &["libc::kill", "kill("],
        "signal-0 liveness observation of a session lock holder",
    ),
    (
        "src/infrastructure/processes/containers/script_stderr.rs",
        &["kill("],
        "the container-script runner ending its own script Child on timeout \
         (#1924 retained `inspect` argv, #2024 S4b create preflight)",
    ),
    (
        "src/infrastructure/tools/grep.rs",
        &["kill("],
        "tool invocation containment: the grep tool's own `rg` Child on a cap",
    ),
    (
        "src/infrastructure/tools/find_fd.rs",
        &["start_kill", "kill_on_drop"],
        "tool invocation containment: the find tool's own `fd` Child on a cap",
    ),
    (
        "src/infrastructure/tools/spawn_container.rs",
        &["process_group("],
        "launch topology only: a detached container launch leads its own group \
         so the supervisor's handle covers it; no signal here",
    ),
    (
        "src/infrastructure/tools/swarm_board_worker.rs",
        &["prctl"],
        "the persistent coordination interpreter binds its own life to the \
         harness in its prelude (PR_SET_PDEATHSIG, the child's own syscall); \
         the harness sends it nothing and closes stdin to end it",
    ),
    (
        "src/infrastructure/tools/swarm_bridge.rs",
        &["/proc/"],
        "signal-free liveness observation (`/proc/<pid>/stat` start time, \
         `/proc/self/ns/pid`) for the coordination store's member records",
    ),
    (
        "src/infrastructure/runtime_identity.rs",
        &["/proc/"],
        "this process's own executable digest via `/proc/<self>/exe`",
    ),
    (
        "src/interface/cli/uds_socket.rs",
        &["/proc/"],
        "the kernel's unix-socket table (`/proc/net/unix`) for socket liveness",
    ),
];

#[test]
fn process_effects_are_exactly_the_allowlisted_files_and_vocabulary() {
    let mut violations = Vec::new();
    let mut seen: BTreeSet<(&str, &str)> = BTreeSet::new();
    for path in production_files() {
        let allowed = PROCESS_EFFECT_ALLOWLIST
            .iter()
            .find(|(file, _, _)| *file == path.as_str());
        for (line_no, line) in production_code(&path) {
            for pattern in PROCESS_EFFECT_PATTERNS {
                if !matches(&line, pattern) {
                    continue;
                }
                match allowed {
                    Some((file, patterns, _)) if patterns.contains(pattern) => {
                        seen.insert((file, pattern));
                    }
                    _ => violations.push(format!(
                        "{path}:{line_no}: `{pattern}` in `{}`",
                        line.trim()
                    )),
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "process effects outside the #1940 allowlist (owned-handle supervisor, bash \
         containment, Python ExecutionScope, signal-0 observation, retained-environment \
         command adapter, tool-child containment, /proc observation):\n{}",
        violations.join("\n")
    );
    // Affirmative and exact: every allowlisted (file, pattern) pair is live.
    let mut stale = Vec::new();
    for (file, patterns, _) in PROCESS_EFFECT_ALLOWLIST {
        assert!(Path::new(file).exists(), "allowlisted file {file} is gone");
        for pattern in *patterns {
            if !seen.contains(&(file, pattern)) {
                stale.push(format!("{file}: `{pattern}`"));
            }
        }
    }
    assert!(
        stale.is_empty(),
        "allowlist entries no longer matched by any production line (remove them):\n{}",
        stale.join("\n")
    );
}

/// The signal-0 observer only ever probes: the one `libc::kill` it makes
/// carries signal 0.
#[test]
fn session_ownership_only_probes_with_signal_zero() {
    let calls: Vec<_> = production_code("src/infrastructure/persistence/session_ownership.rs")
        .into_iter()
        .filter(|(_, line)| line.contains("libc::kill"))
        .collect();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].1.contains(", 0)"), "{}", calls[0].1);
}

/// Subagent, environment and swarm-member teardown never reads a pid: the
/// application capabilities, their domain policy, the interface adapters and
/// the composition graph do not contain the token at all.
const NO_PID_FILES: &[&str] = &[
    "src/domain/subagent_teardown.rs",
    "src/domain/parent_control.rs",
    "src/domain/harness_lifetime.rs",
    "src/domain/environment_retention.rs",
    "src/domain/environment_registry.rs",
    "src/domain/environment_registry/record_lookup.rs",
    "src/domain/environment_registry_inspect.rs",
    "src/application/subagents/mod.rs",
    "src/application/subagents/dto.rs",
    "src/application/subagents/dto/container_config.rs",
    "src/application/subagents/ports.rs",
    "src/application/subagents/standard_script.rs",
    "src/application/environments/mod.rs",
    "src/application/environments/use_cases/mod.rs",
    "src/application/environments/use_cases/list_environments.rs",
    "src/interface/uds/subagent_teardown/mod.rs",
    "src/interface/uds/subagent_teardown/mapping.rs",
    "src/interface/uds/subagent_teardown/wire.rs",
    "src/application/subagents/use_cases/mod.rs",
    "src/application/subagents/use_cases/bounded_settlement.rs",
    "src/application/subagents/use_cases/kill_delegated_agent.rs",
    "src/application/subagents/use_cases/terminate_all_delegated_agents.rs",
    "src/application/subagents/use_cases/settle_delegated_child.rs",
    "src/application/subagents/use_cases/harness_shutdown.rs",
    "src/application/subagents/use_cases/terminate_delegated_agent.rs",
    "src/application/subagents/use_cases/observe_owned_child_exit.rs",
    "src/application/subagents/use_cases/compensate_failed_launch.rs",
    "src/application/subagents/use_cases/select_container_config.rs",
    "src/application/environments/ports.rs",
    "src/application/environments/dto.rs",
    "src/application/environments/dto/assets.rs",
    "src/application/environments/dto/gc.rs",
    "src/application/environments/dto/standard_image.rs",
    "src/application/environments/use_cases/diagnose_container_runtime.rs",
    "src/application/environments/use_cases/list_container_configs.rs",
    "src/application/environments/use_cases/initialise_standard_container.rs",
    "src/application/environments/use_cases/container_status.rs",
    "src/application/environments/use_cases/kill_environment.rs",
    "src/application/environments/use_cases/finalize_environment_member.rs",
    "src/application/environments/use_cases/restore_registry.rs",
    "src/application/environments/use_cases/gc_orphaned_environments.rs",
    "src/application/swarm/mod.rs",
    "src/application/swarm/ports.rs",
    "src/interface/tools/agent_cmd_kill.rs",
    "src/interface/uds/subagent_teardown/controller.rs",
    "src/interface/uds/subagent_teardown/presenter.rs",
    "src/interface/cli/uds_delete_all_subagents.rs",
    "src/interface/cli/uds_busy_subagents.rs",
    "src/interface/cli/uds_shutdown.rs",
    "src/interface/cli/uds_dispatch_session.rs",
    "src/interface/cli/uds_teardown_adapters.rs",
    "src/interface/cli/uds_teardown_handles.rs",
    "src/composition/subagent_teardown.rs",
    "src/composition/subagent_termination.rs",
    "src/composition/subagent_lifecycle.rs",
    "src/composition/environments.rs",
    "src/infrastructure/processes/owned_child_termination.rs",
    "src/infrastructure/processes/direct_child_routing.rs",
    "src/infrastructure/tools/subagent_teardown_registry.rs",
    "src/infrastructure/tools/subagent_teardown_wiring.rs",
    "src/infrastructure/tools/environment_member_shutdown.rs",
    "src/infrastructure/tools/owner_exit.rs",
    "src/infrastructure/tools/retained_environment_teardown.rs",
    "src/infrastructure/tools/swarm_member_termination.rs",
    "src/infrastructure/tools/subagent_cleanup.rs",
    "src/infrastructure/tools/subagent_monitor_exit.rs",
    "src/infrastructure/tools/spawn_reaper.rs",
    "src/infrastructure/tools/spawn_registry.rs",
    "src/infrastructure/tools/process_tree.rs",
];

fn has_pid_token(line: &str) -> bool {
    let bytes = line.as_bytes();
    line.match_indices("pid").any(|(index, _)| {
        let before = index
            .checked_sub(1)
            .map(|i| bytes[i] as char)
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        let after = bytes
            .get(index + 3)
            .map(|&b| b as char)
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        !before && !after
    })
}

#[test]
fn teardown_paths_carry_no_pid() {
    for path in NO_PID_FILES {
        for (line_no, line) in production_code(path) {
            assert!(
                !has_pid_token(&line),
                "{path}:{line_no} names a pid on a teardown path: `{}`",
                line.trim()
            );
        }
    }
    let mut files = Vec::new();
    walk(Path::new("src/application/subagents"), &mut files);
    walk(Path::new("src/application/environments"), &mut files);
    walk(Path::new("src/interface/uds/subagent_teardown"), &mut files);
    for path in files.iter().filter(|p| !p.ends_with("_tests.rs")) {
        assert!(
            NO_PID_FILES.contains(&path.as_str()),
            "{path} is a teardown module not covered by the no-pid ratchet; add it"
        );
    }
}

/// A merged descendant row carries no pid: the merge never reads the
/// snapshot's `pid`, and the only pid the registry publishes on the wire is
/// the display-only pid of a child this harness launched.
#[test]
fn merged_descendants_carry_no_pid_and_the_wire_pid_is_display_only() {
    let merge = production_code("src/infrastructure/tools/subagent_monitor_merge.rs");
    let pid_lines: Vec<_> = merge
        .iter()
        .filter(|(_, line)| has_pid_token(line))
        .map(|(_, line)| line.trim().to_string())
        .collect();
    assert_eq!(
        pid_lines,
        vec!["entry.pid = REPORTED_DESCENDANT_PID;"],
        "the merge must not read `pid` from a snapshot"
    );
    assert!(
        merge
            .iter()
            .any(|(_, line)| line.trim() == "pub const REPORTED_DESCENDANT_PID: u32 = 0;"),
        "a merged row's pid is the named zero"
    );
    assert!(
        !merge.iter().any(|(_, line)| line.contains("get(\"pid\")")),
        "the merge reads no `pid` from a snapshot"
    );
    let cascade = production_code("src/infrastructure/tools/subagent_cascade.rs");
    let pid_lines: Vec<_> = cascade
        .iter()
        .filter(|(_, line)| has_pid_token(line))
        .map(|(_, line)| line.trim().to_string())
        .collect();
    assert_eq!(
        pid_lines,
        vec!["obj.insert(\"pid\".into(), serde_json::json!(entry.pid));"],
        "the cascade only publishes the display-only pid"
    );
    let registry = production_code("src/infrastructure/tools/subagent_registry.rs");
    assert!(
        !registry
            .iter()
            .any(|(_, line)| line.contains("fn request_owned_child_termination")),
        "SubagentEntry::request_owned_child_termination was retired by #1940"
    );
}

/// Names retired across epic #1929 (A–G). None may survive as a definition,
/// a caller, a re-export forwarder, a facade, a duplicate owner or a test.
const RETIRED_NAMES: &[&str] = &[
    "ProcessOwnership",
    "process_ownership",
    "Lease::",
    "same_namespace_descendant_keys",
    "is_same_namespace",
    "retire_reported",
    "prune_retire",
    "verify_persisted_live_subagent",
    "VerifiedLiveSubagent",
    "entry_from_persisted_subagent",
    "confirm_reported_pid_in_our_namespace",
    "terminate_removed_entry",
    "shutdown_all(",
    "shutdown_all_with_count",
    "fn kill_agent",
    "kill_agent(",
    "\"signalled\"",
    "\"unsignalled\"",
    "fn terminate_member",
    "terminate_member(",
    "ScriptEnvironmentKill",
    "EnvironmentFinalizationUseCase",
    "EnvironmentFinalizationPort",
    "EnvironmentControlUseCase",
    "EnvironmentKillPort",
    "environment_control::",
    "environment_control.rs",
    "environment_finalization",
    "request_owned_child_termination",
    "terminate_owned_process_tree",
    "sigterm_pid",
    "terminate_local_process_group",
    "reset_subagent_roster_on_restore",
    "release_departing_entry",
    "delete_all_subagents_from_registry",
    "busy_reader_intercept_with_registry",
    "shutdown_on_with",
    "TeardownSpawner",
    "cleanup::FinalizeMode",
    "NoMemberShutdown",
    "restored_pid",
    "RuntimeIdentity { pid",
];

/// Files that legitimately name retired identifiers: the ratchets themselves.
const RETIRED_NAME_SCAN_EXEMPT: &[&str] = &["tests/architecture.rs", "tests/architecture/"];

const RETIRED_FILES: &[&str] = &[
    "src/infrastructure/tools/process_ownership.rs",
    "src/infrastructure/tools/process_ownership_cov_tests.rs",
    "src/infrastructure/tools/subagent_monitor_merge_ownership_tests.rs",
    "src/infrastructure/tools/spawn_registry_ownership_tests.rs",
    "src/infrastructure/tools/environment_kill.rs",
    "src/application/environment_control.rs",
    "src/domain/environment_finalization.rs",
    "tests/contracts/environment_kill_port.rs",
    "tests/contracts/environment_finalization_port.rs",
];

#[test]
fn retired_teardown_names_and_files_do_not_exist() {
    for file in RETIRED_FILES {
        assert!(
            !Path::new(file).exists(),
            "{file} was retired by epic #1929"
        );
    }
    let mut files = Vec::new();
    walk(Path::new("src"), &mut files);
    walk(Path::new("tests"), &mut files);
    for entry in fs::read_dir("tests/features").expect("features") {
        let path = entry.expect("feature").path();
        if path.extension().is_some_and(|ext| ext == "feature") {
            files.push(path.to_string_lossy().to_string());
        }
    }
    let mut hits = Vec::new();
    for path in files {
        if RETIRED_NAME_SCAN_EXEMPT
            .iter()
            .any(|exempt| path.starts_with(exempt))
        {
            continue;
        }
        let source = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
        for (line_no, line) in source.lines().enumerate() {
            for name in RETIRED_NAMES {
                if line.contains(name) {
                    hits.push(format!("{path}:{}: `{name}`", line_no + 1));
                }
            }
        }
    }
    assert!(
        hits.is_empty(),
        "retired epic #1929 names survive (no facade, forwarder, duplicate owner or \
         sole-purpose test may remain):\n{}",
        hits.join("\n")
    );
}

/// One owner per teardown responsibility: each is a single production
/// definition, and no second definition of the same responsibility exists.
const SINGLE_OWNERS: &[(&str, &str, &str)] = &[
    (
        "direct unreaped Child handle and TERM/KILL fallback",
        "src/infrastructure/processes/owned_child_supervisor.rs",
        "struct OwnedChildSupervisor",
    ),
    (
        "selected termination (agent_cmd kill, forwarded terminate_delegated_agent)",
        "src/application/subagents/use_cases/kill_delegated_agent.rs",
        "struct KillDelegatedAgent",
    ),
    (
        "owner conclusion of a selected direct child",
        "src/application/subagents/use_cases/terminate_delegated_agent.rs",
        "struct TerminateDelegatedAgent",
    ),
    (
        "whole-fleet teardown (delete-all, signals, last client, session transitions, common shutdown)",
        "src/application/subagents/use_cases/terminate_all_delegated_agents.rs",
        "struct TerminateAllDelegatedAgents",
    ),
    (
        "per-child sweep settlement (fleet and environment kill)",
        "src/application/subagents/use_cases/settle_delegated_child.rs",
        "struct SettleDelegatedChild",
    ),
    (
        "common harness shutdown drive",
        "src/application/subagents/use_cases/harness_shutdown.rs",
        "struct ExecuteHarnessShutdown",
    ),
    (
        "exit observation of an owned child",
        "src/application/subagents/use_cases/observe_owned_child_exit.rs",
        "struct ObserveOwnedChildExit",
    ),
    (
        "launch rollback",
        "src/application/subagents/use_cases/compensate_failed_launch.rs",
        "struct CompensateFailedLaunch",
    ),
    (
        "environment kill (kill_container)",
        "src/application/environments/use_cases/kill_environment.rs",
        "struct KillEnvironment",
    ),
    (
        "environment member finalization and #1924 retention",
        "src/application/environments/use_cases/finalize_environment_member.rs",
        "struct FinalizeEnvironmentMember",
    ),
    (
        "swarm member termination by delegation",
        "src/infrastructure/tools/swarm_member_termination.rs",
        "struct DelegatedSwarmMemberTermination",
    ),
    (
        "registry claim ladder and compensation",
        "src/infrastructure/tools/subagent_teardown_registry.rs",
        "struct RegistryDelegatedAgents",
    ),
    (
        "child parent-loss binding",
        "src/interface/cli/uds_parent_control.rs",
        "struct ConnectionTeardown",
    ),
];

/// `line` declares exactly `declaration` (`struct X` / `enum X` / `fn X` /
/// `trait X` / `type X`) under ANY visibility — `pub`, `pub(crate)`,
/// `pub(super)`, `pub(in …)` or private — and not a longer identifier.
fn declares(line: &str, declaration: &str) -> bool {
    let mut rest = line.trim_start();
    if let Some(after) = rest.strip_prefix("pub") {
        rest = match after.strip_prefix('(') {
            Some(scoped) => scoped.split_once(')').map(|(_, r)| r).unwrap_or(""),
            None => after,
        }
        .trim_start();
    }
    rest.strip_prefix(declaration).is_some_and(|tail| {
        !tail
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

#[test]
fn declares_matches_every_visibility_and_only_exact_names() {
    for line in [
        "pub struct KillDelegatedAgent {",
        "pub(crate) struct KillDelegatedAgent {",
        "pub(super) struct KillDelegatedAgent<'a> {",
        "pub(in crate::x) struct KillDelegatedAgent;",
        "    struct KillDelegatedAgent {",
    ] {
        assert!(declares(line, "struct KillDelegatedAgent"), "{line}");
    }
    assert!(!declares(
        "pub struct KillDelegatedAgentTool {",
        "struct KillDelegatedAgent"
    ));
    assert!(!declares(
        "pub enum KillDelegatedAgent {",
        "struct KillDelegatedAgent"
    ));
}

#[test]
fn each_teardown_responsibility_has_exactly_one_owner() {
    let files = production_files();
    for (responsibility, owner_file, declaration) in SINGLE_OWNERS {
        let owner = production_code(owner_file);
        assert!(
            owner.iter().any(|(_, line)| declares(line, declaration)),
            "{owner_file} must declare `{declaration}` ({responsibility})"
        );
        for path in &files {
            if path == owner_file {
                continue;
            }
            assert!(
                !production_code(path)
                    .iter()
                    .any(|(_, line)| declares(line, declaration)),
                "{path} declares a second `{declaration}` ({responsibility})"
            );
        }
    }
}
