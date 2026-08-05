//! Cascade-removal of dead sub-agent sub-trees and survivor-set serialization
//! (#831).
//!
//! When an agent exits or is killed its whole sub-tree is dead, and the root
//! registry (and the TUI panel that mirrors it) must drop the agent AND every
//! transitive descendant promptly. This module owns the registry-pruning walk,
//! the orphaned-process cleanup, and the canonical `subagent_state_changed`
//! serialization shared by the kill/reaper broadcast paths and the descendant
//! forwarding path (`subagent_monitor`).

use std::collections::HashMap;

use super::subagent_registry::{SubagentEntry, SubagentRegistry};

/// Outcome of a cascade-remove on the registry (#831).
pub struct CascadeOutcome {
    /// The `(id, entry)` pairs removed (the dead agent + all its descendants),
    /// returned so the caller can terminate their OS processes / abort monitors
    /// before they fall out of tracking (security review).
    pub removed: Vec<(String, SubagentEntry)>,
    /// Canonical survivors-only `subagent_state_changed` event to broadcast, or
    /// `None` when nothing was removed (unknown agent) so the caller skips a
    /// redundant broadcast.
    pub event: Option<String>,
}

/// Remove `agent_id` AND every transitive descendant (by `parent_id`) from the
/// registry, returning the removed `(id, entry)` pairs (#831).
///
/// When an agent exits or is killed its whole sub-tree is dead: a grandchild
/// whose parent is gone can never make progress and must not linger in the root
/// registry (the lingering-panel bug). Unrelated sibling trees are untouched. A
/// missing `agent_id` is a no-op and returns an empty Vec.
///
/// The removed entries are returned (not just ids) so callers can terminate the
/// orphaned OS processes / abort monitor tasks before they fall out of the
/// registry — see [`terminate_removed_entry`] (#831 security review).
pub fn cascade_remove(registry: &SubagentRegistry, agent_id: &str) -> Vec<(String, SubagentEntry)> {
    let mut guard = registry.lock().unwrap_or_else(|e| e.into_inner());
    cascade_remove_locked(&mut guard, agent_id)
}

/// [`cascade_remove`] operating on an already-held registry guard, so a caller
/// can prune the dead sub-tree AND snapshot the survivors in a SINGLE critical
/// section (perf review: avoids re-locking + the prune/snapshot race window).
pub fn cascade_remove_locked(
    guard: &mut HashMap<String, SubagentEntry>,
    agent_id: &str,
) -> Vec<(String, SubagentEntry)> {
    if !guard.contains_key(agent_id) {
        return Vec::new();
    }
    // Build a parent_id -> children adjacency map ONCE (O(N)) so the cascade is
    // O(N) total rather than re-scanning the whole registry per removed node
    // (perf review). Computed up front; the tree shape does not change during
    // the removal walk.
    let mut children: HashMap<String, Vec<String>> = HashMap::new();
    for (id, entry) in guard.iter() {
        if let Some(parent) = &entry.parent_id {
            children.entry(parent.clone()).or_default().push(id.clone());
        }
    }
    let mut removed = Vec::new();
    let mut frontier = vec![agent_id.to_string()];
    while let Some(id) = frontier.pop() {
        let Some(entry) = guard.remove(&id) else {
            continue;
        };
        if let Some(kids) = children.get(&id) {
            frontier.extend(kids.iter().cloned());
        }
        removed.push((id, entry));
    }
    removed
}

/// Best-effort terminate a cascade-removed entry's OS process and monitor task so
/// descendants do not linger as orphaned, untracked processes after their
/// ancestor is killed/exits (#831 security review). Aborts the monitor task (if
/// any) and SIGTERMs the pid (if any). Does NOT touch the registry — the entry
/// has already been removed — and does NOT send await signals (the caller owns
/// that, as the signal semantics differ between the kill and reaper paths).
pub fn terminate_removed_entry(entry: &SubagentEntry) {
    if let Some(ref handle) = entry.monitor_handle {
        handle.abort();
    }
    if entry.pid != 0 {
        sigterm_pid(entry.pid);
    }
}

/// Best-effort SIGTERM a pid via a direct syscall. Avoids fork+exec of `kill(1)`
/// and its blocking `.status()` wait, which would stall a tokio worker for every
/// subagent in the tree. A stale/dead pid simply yields ESRCH, which we ignore.
///
/// `libc` is a unix-only dependency (see Cargo.toml + bash/mod.rs), so this is a
/// no-op on non-unix hosts, where subagents are not spawned. `i32::try_from`
/// guards the `u32 -> pid_t` narrowing so an out-of-range pid can never wrap
/// negative and turn into a process-group signal (Linux pid_max keeps this
/// unreachable, but the cast is a footgun worth closing).
pub(crate) fn sigterm_pid(pid: u32) {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(pid) {
        // SAFETY: FFI call to `libc::kill` with an owned pid and a constant signal.
        unsafe {
            libc::kill(pid, libc::SIGTERM);
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
}

/// Cascade-remove `agent_id`'s dead sub-tree from the registry and, if anything
/// was actually removed, return both the removed entries (for process cleanup)
/// and a canonical `subagent_state_changed` event carrying the SURVIVORS only —
/// ready to broadcast so every connected client (the TUI panel) drops the dead
/// agents promptly (#831).
///
/// Prune and survivors-snapshot happen under a SINGLE registry lock (perf
/// review): no re-lock, and no window for another thread to mutate the roster
/// between the removal and the snapshot. A still-live agent is never touched.
pub fn cascade_remove_and_state_changed(
    registry: &SubagentRegistry,
    agent_id: &str,
) -> CascadeOutcome {
    let mut guard = registry.lock().unwrap_or_else(|e| e.into_inner());
    let removed = cascade_remove_locked(&mut guard, agent_id);
    let event = if removed.is_empty() {
        None
    } else {
        Some(build_state_changed_event_locked(&guard))
    };
    CascadeOutcome { removed, event }
}

/// Build a canonical `subagent_state_changed` event (camelCase per
/// `SubagentInfo`) from the registry's current entries, sorted by id for stable
/// ordering. Projects to KNOWN fields only — no arbitrary child-supplied keys
/// cross onto the parent stream (trust-boundary pattern, review).
///
/// The returned line is `\n`-TERMINATED, ready to broadcast as-is. The
/// protocol frames events by newline; a send site that forgot to append one
/// made the client writer splice the event onto the NEXT line, so ancestors'
/// monitors read one unparseable blob and dropped BOTH events — grandchildren
/// spawned after a merge never propagated and exited ghosts were never pruned
/// (#1055). Terminating here makes every send site correct by construction.
pub fn build_state_changed_event(registry: &SubagentRegistry) -> String {
    let guard = registry.lock().unwrap_or_else(|e| e.into_inner());
    build_state_changed_event_locked(&guard)
}

/// [`build_state_changed_event`] operating on an already-held registry guard, so
/// a caller can prune a dead sub-tree and serialize the survivors in ONE critical
/// section (perf review).
pub fn build_state_changed_event_locked(guard: &HashMap<String, SubagentEntry>) -> String {
    let subagents: Vec<serde_json::Value> = {
        let mut entries: Vec<(&String, &SubagentEntry)> = guard.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        entries
            .into_iter()
            .map(|(id, entry)| {
                let mut obj = serde_json::Map::new();
                let display_name = entry.effective_display_name(id);
                obj.insert("agentId".into(), serde_json::json!(display_name));
                obj.insert("displayName".into(), serde_json::json!(display_name));
                obj.insert("agentUuid".into(), serde_json::json!(entry.agent_uuid));
                obj.insert(
                    "status".into(),
                    serde_json::json!(entry.status.to_wire_str()),
                );
                if let Some(tool) = &entry.last_tool {
                    obj.insert("lastTool".into(), serde_json::json!(tool));
                }
                if let Some(err) = &entry.last_error {
                    obj.insert("lastError".into(), serde_json::json!(err));
                }
                obj.insert("pid".into(), serde_json::json!(entry.pid));
                obj.insert("readOnly".into(), serde_json::json!(entry.read_only));
                obj.insert(
                    "socketPath".into(),
                    serde_json::json!(entry.socket_path.to_string_lossy()),
                );
                if let Some(parent) = &entry.parent_id {
                    obj.insert("parentId".into(), serde_json::json!(parent));
                }
                obj.insert(
                    "runtimeBackend".into(),
                    serde_json::json!(entry.runtime_backend),
                );
                if let Some(v) = &entry.container_uuid {
                    obj.insert("containerUuid".into(), serde_json::json!(v));
                }
                if let Some(v) = &entry.container_ref {
                    obj.insert("containerRef".into(), serde_json::json!(v));
                }
                if let Some(v) = &entry.container_name {
                    obj.insert("containerName".into(), serde_json::json!(v));
                }
                if let Some(v) = &entry.repo_url {
                    obj.insert("repoUrl".into(), serde_json::json!(v));
                }
                if let Some(v) = &entry.environment_id {
                    obj.insert("environmentId".into(), serde_json::json!(v));
                }
                if let Some(v) = &entry.environment_health {
                    obj.insert("environmentHealth".into(), serde_json::json!(v));
                }
                if let Some(v) = &entry.socket_mode {
                    obj.insert("socketMode".into(), serde_json::json!(v));
                }
                if let Some(v) = &entry.workspace_path {
                    obj.insert("workspacePath".into(), serde_json::json!(v));
                }
                if let Some(workflow) = &entry.workflow {
                    if let Ok(w) = serde_json::to_value(workflow) {
                        obj.insert("workflow".into(), w);
                    }
                }
                serde_json::Value::Object(obj)
            })
            .collect()
    };
    let mut line = serde_json::json!({
        "type": "subagent_state_changed",
        "subagents": subagents,
    })
    .to_string();
    line.push('\n');
    line
}

#[cfg(test)]
#[path = "subagent_cascade_cov_tests.rs"]
mod cov_tests;
#[cfg(test)]
#[path = "subagent_cascade_tests.rs"]
mod tests;
