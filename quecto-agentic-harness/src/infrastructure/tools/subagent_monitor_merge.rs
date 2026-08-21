// Descendant subagent merge/forward logic for the persistent monitor (#815,
// #831). Split out of `subagent_monitor.rs` to respect the per-file line cap.
//
// Architecture: infrastructure layer — MUST NOT import `crate::interface`.

use std::time::Instant;

use super::subagent_lifecycle::SubagentLifecycleState;
use super::subagent_registry::{SubagentEntry, SubagentRegistry, SubagentStatus};

/// Maximum number of descendant entries accepted from a single child's
/// `subagent_state_changed` event (#815 security review). A child sits inside
/// the same-user trust boundary, but this is a new re-broadcast path: capping
/// the merged count stops a misbehaving/compromised child from injecting an
/// unbounded number of fabricated descendants into the root registry (which is
/// then re-serialized at every ancestor hop).
const MAX_FORWARDED_SUBAGENTS: usize = 256;

/// Merge the descendants from a child's `subagent_state_changed` `value` into
/// the registry (preserving each entry's REAL `agentId`/`parentId`), then build
/// a single canonical `subagent_state_changed` event carrying the WHOLE current
/// registry — the union of the root's own children and all merged descendants
/// (#815, architecture review).
///
/// Why merge instead of forwarding the grandchildren-only list verbatim: the
/// consumer (`update_subagent_bar`) and `build_subagent_info_list` polling both
/// treat each `subagent_state_changed` as a FULL replace. A partial push that
/// listed only grandchildren would evict the root's direct children (and vice
/// versa, the root's own push would evict the grandchildren), so the bar would
/// oscillate and grandchildren would never appear stably. Carrying the union on
/// every push fixes that and keeps `get_subagents` polling in agreement.
///
/// Each descendant keeps its authoritative identity — never re-stamped to the
/// immediate child's id, which would mis-attribute grandchildren — so an
/// already-forwarded great-grandchild entry chains up to arbitrary depth.
/// Returns `None` for any value that is not a `subagent_state_changed` event.
pub fn merge_and_forward_state_changed(
    value: &serde_json::Value,
    registry: &SubagentRegistry,
    forwarding_child_id: &str,
) -> Option<String> {
    if value.get("type").and_then(|t| t.as_str()) != Some("subagent_state_changed") {
        return None;
    }
    if let Some(descendants) = value.get("subagents").and_then(|v| v.as_array()) {
        merge_descendants(registry, forwarding_child_id, descendants);
    }
    Some(super::subagent_cascade::build_state_changed_event(registry))
}

/// Merge descendant `SubagentInfo` entries (camelCase wire fields) into the
/// registry as a SCOPED REPLACE of `forwarding_child_id`'s sub-tree.
///
/// A child's `subagent_state_changed` push is the AUTHORITATIVE, full snapshot of
/// everything below that child. So beyond upserting the pushed entries (bounded
/// by [`MAX_FORWARDED_SUBAGENTS`]), we prune any registry entry that is a
/// transitive descendant of `forwarding_child_id` but ABSENT from this push —
/// i.e. a grandchild that exited or was killed under the child. Without this, the
/// pure-upsert merge could never remove a dead grandchild from the root registry
/// (it stops being forwarded once gone), so it lingered in the TUI panel forever
/// (#831). Entries outside the forwarding child's sub-tree (the root's own
/// children, sibling trees) are never touched, preserving the full-replace
/// stability that #815 relies on.
fn merge_descendants(
    registry: &SubagentRegistry,
    forwarding_child_id: &str,
    descendants: &[serde_json::Value],
) {
    if descendants.len() > MAX_FORWARDED_SUBAGENTS {
        tracing::warn!(
            count = descendants.len(),
            cap = MAX_FORWARDED_SUBAGENTS,
            "monitor: truncating forwarded descendant list over cap"
        );
    }
    let mut guard = registry.lock().unwrap_or_else(|e| e.into_inner());
    // A child launched inside a script-managed environment sees its own local
    // descendants' sockets, but an ancestor outside that environment does not.
    // The descendants correctly report `executionBackend: local` relative to
    // their immediate parent, so reachability must also account for the
    // forwarding parent's environment boundary.
    let forwarding_child_crosses_environment =
        guard.get(forwarding_child_id).is_some_and(|entry| {
            entry.environment_ref.is_some() || entry.forwarded_environment.is_some()
        });
    let mut pushed_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut next_sequence = guard
        .values()
        .map(|entry| entry.notification_sequence)
        .max()
        .unwrap_or(0)
        .saturating_add(1);
    for d in descendants.iter().take(MAX_FORWARDED_SUBAGENTS) {
        // Prefer additive agentUuid for durable registry keys; wire agentId is
        // the display label only (#1378). Fall back to agentId for legacy
        // snapshots that predate dual identity.
        let display_label = d
            .get("displayName")
            .and_then(|v| v.as_str())
            .or_else(|| d.get("agentId").and_then(|v| v.as_str()))
            .unwrap_or("");
        if display_label.is_empty() && d.get("agentUuid").and_then(|v| v.as_str()).is_none() {
            continue;
        }
        let registry_key = d
            .get("agentUuid")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                d.get("agentId")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| crate::domain::ids::AgentUuid::mint().into_string());
        let display_name = if display_label.is_empty() {
            registry_key.clone()
        } else {
            display_label.to_string()
        };
        pushed_ids.insert(registry_key.clone());
        let socket_path = if forwarded_script_descendant_socket_is_ancestor_local(d)
            || (forwarding_child_crosses_environment
                && d.get("executionBackend").and_then(|v| v.as_str())
                    == Some(super::subagent_environment_wire::BACKEND_LOCAL))
        {
            std::path::PathBuf::new()
        } else {
            d.get("socketPath")
                .and_then(|v| v.as_str())
                .map(std::path::PathBuf::from)
                .unwrap_or_default()
        };
        let pid = d.get("pid").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let agent_uuid = crate::domain::ids::AgentUuid::new(registry_key.clone());
        let entry = guard.entry(registry_key.clone()).or_insert_with(|| {
            SubagentEntry::with_identity(agent_uuid, display_name.clone(), socket_path.clone(), pid)
        });
        // Keep identity fields authoritative from the child's snapshot.
        entry.agent_uuid = crate::domain::ids::AgentUuid::new(registry_key.clone());
        entry.display_name = display_name;
        if let Some(status) = d
            .get("status")
            .and_then(|v| v.as_str())
            .map(SubagentStatus::from_wire_str)
        {
            entry.lifecycle = SubagentLifecycleState::from_status(&status);
            entry.status = status;
        }
        entry.last_tool = d
            .get("lastTool")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        entry.last_error = d
            .get("lastError")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        entry.pid = pid;
        entry.socket_path = socket_path;
        entry.parent_id = d
            .get("parentId")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| Some(forwarding_child_id.to_string()));
        entry.workflow = d
            .get("workflow")
            .and_then(|w| serde_json::from_value(w.clone()).ok());
        entry.read_only = d.get("readOnly").and_then(|v| v.as_bool()).unwrap_or(false);
        // #1369 slice 4: retain the child's reported execution backend and
        // typed environment object so the re-broadcast union preserves them.
        entry.forwarded_execution_backend = d
            .get("executionBackend")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        entry.forwarded_environment = d
            .get("environment")
            .and_then(|e| serde_json::from_value(e.clone()).ok());
        entry.notification_sequence = next_sequence;
        next_sequence = next_sequence.saturating_add(1);
        entry.updated_at = Instant::now();
    }

    // Scoped prune: drop transitive descendants of the forwarding child that the
    // authoritative push omitted (they died under the child). Computed AFTER the
    // upsert so re-parented entries chain correctly. Only the forwarding child's
    // sub-tree is in scope; the child itself and all other trees are untouched.
    let stale: Vec<String> = transitive_descendants(&guard, forwarding_child_id)
        .into_iter()
        .filter(|id| !pushed_ids.contains(id))
        .collect();
    for id in stale {
        guard.remove(&id);
    }
}

fn forwarded_script_descendant_socket_is_ancestor_local(d: &serde_json::Value) -> bool {
    d.get("executionBackend").and_then(|v| v.as_str())
        == Some(super::subagent_environment_wire::BACKEND_SCRIPT)
        && d.get("environment").is_some()
        && d.pointer("/environment/socketMode")
            .and_then(|v| v.as_str())
            != Some("proxy")
}

/// Collect the ids of every transitive descendant of `root` (by `parent_id`) in
/// the registry, NOT including `root` itself. Used to scope the forwarded
/// full-replace prune to one child's sub-tree (#831).
fn transitive_descendants(
    guard: &std::collections::HashMap<String, SubagentEntry>,
    root: &str,
) -> Vec<String> {
    let mut children: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    for (id, entry) in guard.iter() {
        if let Some(parent) = &entry.parent_id {
            children.entry(parent.as_str()).or_default().push(id);
        }
    }
    let mut out = Vec::new();
    let mut frontier: Vec<&str> = children.get(root).cloned().unwrap_or_default();
    while let Some(id) = frontier.pop() {
        out.push(id.to_string());
        if let Some(kids) = children.get(id) {
            frontier.extend(kids.iter().copied());
        }
    }
    out
}

/// Line-based wrapper around [`merge_and_forward_state_changed`]: cheap
/// substring pre-filter, then parse once. Returns `None` for non-state lines.
pub fn forward_child_state_changed(
    line: &str,
    registry: &SubagentRegistry,
    forwarding_child_id: &str,
) -> Option<String> {
    if !line.contains("\"type\":\"subagent_state_changed\"") {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    merge_and_forward_state_changed(&value, registry, forwarding_child_id)
}

#[cfg(test)]
#[path = "subagent_monitor_merge_cov_tests.rs"]
mod cov_tests;
