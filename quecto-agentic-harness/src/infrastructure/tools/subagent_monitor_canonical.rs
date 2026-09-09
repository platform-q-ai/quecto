//! Canonical re-stamping of child events forwarded onto the parent's event
//! stream (PRD Stage B / R-B2, #797, #869c, #1060). Split from
//! `subagent_monitor.rs` for the file-size cap.

/// If `line` is a child's `workflow_state` event, re-stamp it with the child's
/// identity so it can be forwarded onto the parent's event stream (PRD Stage B
/// / R-B2): a parent/supervisor then sees descendant workflows without polling
/// each child socket. Returns the re-tagged JSON line, or `None` for any line
/// that is not a `workflow_state` event.
pub fn canonical_workflow_forward(
    value: &serde_json::Value,
    child_id: &str,
    parent_id: Option<&str>,
) -> Option<String> {
    if value.get("type").and_then(|t| t.as_str()) != Some("workflow_state") {
        return None;
    }
    // Re-build a canonical event from KNOWN fields (we do NOT pass through
    // arbitrary child-supplied keys). PRESERVE an existing descendant identity
    // when the event is already a forwarded grandchild workflow (#869c) — only
    // stamp the immediate child's id/parent when the event carries none — so a
    // grandchild's identity is not collapsed into the ancestor moving up the tree.
    let agent = value
        .get("agent_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(child_id);
    let parent = value
        .get("parent_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| parent_id.map(str::to_string));
    let canonical = serde_json::json!({
        "type": "workflow_state",
        "agent_id": agent,
        "parent_id": parent,
        "mode": value.get("mode").cloned().unwrap_or(serde_json::Value::Null),
        "progress": value.get("progress").cloned().unwrap_or(serde_json::Value::Null),
    });
    serde_json::to_string(&canonical).ok()
}

/// Re-stamp a child's `subagent_messages_appended` with child/parent ids (#797)
/// and preserve messageRefs (#1060). Returns `None` if `value` is not that type.
#[rustfmt::skip]
pub fn canonical_messages_appended_forward(
    value: &serde_json::Value,
    child_id: &str,
    parent_id: Option<&str>,
) -> Option<String> {
    if value.get("type").and_then(|t| t.as_str()) != Some("subagent_messages_appended") {
        return None;
    }
    // #1060: prefer messageRefs; drop full messages when refs present.
    let empty = serde_json::json!([]);
    let refs = value.get("messageRefs").cloned().unwrap_or_else(|| empty.clone());
    let msgs = if refs.as_array().is_some_and(|a| !a.is_empty()) {
        empty
    } else {
        value.get("messages").cloned().unwrap_or(empty)
    };
    serde_json::to_string(&serde_json::json!({
        "type": "subagent_messages_appended", "agent_id": child_id,
        "parent_id": parent_id, "messages": msgs, "messageRefs": refs,
    })).ok()
}

/// Line-based wrapper around [`canonical_messages_appended_forward`].
pub fn forward_child_messages_appended(
    line: &str,
    child_id: &str,
    parent_id: Option<&str>,
) -> Option<String> {
    if !line.contains("\"type\":\"subagent_messages_appended\"") {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    canonical_messages_appended_forward(&value, child_id, parent_id)
}

/// Line-based wrapper around [`canonical_workflow_forward`].
pub fn forward_child_workflow_event(
    line: &str,
    child_id: &str,
    parent_id: Option<&str>,
) -> Option<String> {
    if !line.contains("\"type\":\"workflow_state\"") {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    canonical_workflow_forward(&value, child_id, parent_id)
}

/// Bound on forwarded quota groups; a child's configuration already bounds
/// them, this guards the parent's stream against a hostile or buggy child.
const MAX_FORWARDED_GROUPS: usize = 32;

/// If `line` is a child's `admission_state_changed` (#1679 P4), re-stamp it
/// with the child's identity (preserving a forwarded grandchild's, like
/// workflow events) and rebuild the bounded `admission` view from KNOWN
/// fields only, so a supervisor sees a descendant waiting for admission from
/// one socket. Returns `None` for any other line.
pub fn canonical_admission_forward(
    value: &serde_json::Value,
    child_id: &str,
    parent_id: Option<&str>,
) -> Option<String> {
    if value.get("type").and_then(|t| t.as_str()) != Some("admission_state_changed") {
        return None;
    }
    let agent = value
        .get("agent_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(child_id);
    let parent = value
        .get("parent_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| parent_id.map(str::to_string));
    let admission = value.get("admission")?;
    let number = |key: &str| admission.get(key).and_then(|v| v.as_u64());
    let groups: Vec<serde_json::Value> = admission
        .get("groups")
        .and_then(|g| g.as_array())
        .map(|groups| {
            groups
                .iter()
                .take(MAX_FORWARDED_GROUPS)
                .filter_map(|group| {
                    let name = group.get("group")?.as_str()?;
                    let mut out = serde_json::json!({ "group": name });
                    if let Some(cooldown) = group.get("cooldown") {
                        let mut c = serde_json::json!({
                            "state": cooldown.get("state").and_then(|s| s.as_str()).unwrap_or("unknown")
                        });
                        if let Some(remaining) =
                            cooldown.get("remainingSeconds").and_then(|r| r.as_u64())
                        {
                            c["remainingSeconds"] = serde_json::json!(remaining);
                        }
                        out["cooldown"] = c;
                    }
                    Some(out)
                })
                .collect()
        })
        .unwrap_or_default();
    let mut view = serde_json::json!({
        "waiting": number("waiting").unwrap_or(0),
        "admitted": number("admitted").unwrap_or(0),
        "groups": groups,
        "hidden": number("hidden").unwrap_or(0),
        "revision": number("revision").unwrap_or(0),
    });
    if let Some(longest) = number("longestWaitSeconds") {
        view["longestWaitSeconds"] = serde_json::json!(longest);
    }
    if let Some(counters) = admission.get("counters").and_then(|c| c.as_object()) {
        let counter = |key: &str| counters.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
        view["counters"] = serde_json::json!({
            "completed": counter("completed"),
            "refused": counter("refused"),
            "cancelled": counter("cancelled"),
            "abandoned": counter("abandoned"),
        });
    }
    serde_json::to_string(&serde_json::json!({
        "type": "admission_state_changed",
        "agent_id": agent,
        "parent_id": parent,
        "admission": view,
    }))
    .ok()
}
