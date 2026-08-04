use super::*;
use crate::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentStatus, new_registry,
};

fn add(
    registry: &crate::infrastructure::tools::subagent_registry::SubagentRegistry,
    id: &str,
    parent: Option<&str>,
) {
    let mut entry = SubagentEntry::new(format!("/tmp/{id}.sock").into(), 1);
    entry.parent_id = parent.map(str::to_owned);
    registry.lock().unwrap().insert(id.to_string(), entry);
}

#[test]
fn merge_descendants_upserts_updates_and_scoped_prunes_omitted_descendants() {
    let registry = new_registry();
    add(&registry, "child", None);
    add(&registry, "old-grand", Some("child"));
    add(&registry, "sibling", None);
    let event = serde_json::json!({"type":"subagent_state_changed","subagents":[{
        "agentId":"grand","parentId":"child","status":"running","lastTool":"bash",
        "lastError":"none","pid":42,"socketPath":"/tmp/grand.sock","readOnly":true,
        "workflow":{"mode":"active","steps_completed":1,"steps_total":2}
    }]});

    let forwarded = merge_and_forward_state_changed(&event, &registry, "child").unwrap();
    assert!(forwarded.ends_with('\n'));
    let guard = registry.lock().unwrap();
    // Legacy snapshots without agentUuid still key by agentId for back-compat.
    assert!(guard.contains_key("grand"));
    assert!(guard.contains_key("sibling"));
    assert!(!guard.contains_key("old-grand"));
    assert_eq!(guard["grand"].status, SubagentStatus::Running);
    assert_eq!(
        guard["grand"].lifecycle,
        crate::infrastructure::tools::subagent_lifecycle::SubagentLifecycleState::Busy
    );
    assert_eq!(guard["grand"].pid, 42);
    assert_eq!(guard["grand"].workflow.as_ref().unwrap().steps_total, 2);
}

/// #1378: nested merge prefers additive agentUuid as the durable registry key
/// and keeps wire agentId / displayName as the human label.
#[test]
fn merge_descendants_keys_by_agent_uuid_and_keeps_display_label() {
    let registry = new_registry();
    let child_uuid = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    add(&registry, child_uuid, None);
    let grand_uuid = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    let event = serde_json::json!({"type":"subagent_state_changed","subagents":[{
        "agentId":"grand-label",
        "displayName":"grand-label",
        "agentUuid": grand_uuid,
        "parentId": child_uuid,
        "status":"running",
        "pid":99,
        "socketPath":"/tmp/grand.sock"
    }]});

    merge_and_forward_state_changed(&event, &registry, child_uuid).unwrap();
    let guard = registry.lock().unwrap();
    assert!(
        guard.contains_key(grand_uuid),
        "nested entry must be keyed by agentUuid, got keys: {:?}",
        guard.keys().collect::<Vec<_>>()
    );
    assert!(
        !guard.contains_key("grand-label"),
        "must not key nested entries by display label"
    );
    assert_eq!(guard[grand_uuid].display_name, "grand-label");
    assert_eq!(guard[grand_uuid].agent_uuid.as_str(), grand_uuid);
    assert_eq!(
        guard[grand_uuid].parent_id.as_deref(),
        Some(child_uuid),
        "parent_id must stay the forwarding child's durable id"
    );
}

#[test]
fn merge_ignores_non_state_and_line_wrapper_rejects_bad_input() {
    let registry = new_registry();
    assert!(
        merge_and_forward_state_changed(&serde_json::json!({"type":"token"}), &registry, "child")
            .is_none()
    );
    assert!(forward_child_state_changed("not json", &registry, "child").is_none());
    assert!(forward_child_state_changed(r#"{"type":"other"}"#, &registry, "child").is_none());
}

#[test]
fn merge_caps_large_descendant_lists() {
    let registry = new_registry();
    add(&registry, "child", None);
    let descendants: Vec<_> = (0..300)
        .map(|i| serde_json::json!({"agentId":format!("g{i}"),"parentId":"child"}))
        .collect();
    let event = serde_json::json!({"type":"subagent_state_changed","subagents":descendants});
    merge_and_forward_state_changed(&event, &registry, "child").unwrap();
    let count = registry
        .lock()
        .unwrap()
        .keys()
        .filter(|id| id.starts_with('g'))
        .count();
    assert_eq!(count, 256);
}
