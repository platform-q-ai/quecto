//! #815: forward descendant `subagent_state_changed` up the monitor chain.
//!
//! Split out of `subagent_monitor_tests.rs` to keep each test file under the
//! repo's line-count cap. Covers identity preservation, the union (full-replace)
//! semantics that keep siblings from being evicted, multi-hop chaining, the
//! descendant-count cap, and the end-to-end handler path.

use super::*;
use std::path::PathBuf;

fn test_entry() -> SubagentEntry {
    SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0)
}

// Find a descendant entry in a forwarded `subagent_state_changed` array.
fn find_subagent<'a>(v: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
    v["subagents"]
        .as_array()
        .expect("subagents array")
        .iter()
        .find(|s| s["agentId"] == id)
        .unwrap_or_else(|| panic!("missing {id} in forwarded event"))
}

#[test]
fn forward_child_state_changed_preserves_descendant_identity() {
    // A child's subagent_state_changed lists its OWN children (grandchildren of
    // the root). The forward MUST preserve each entry's real agentId/parentId —
    // it must NOT re-stamp them to the immediate child's id (#815).
    let registry = super::super::subagent_registry::new_registry();
    let line = r#"{"type":"subagent_state_changed","subagents":[{"agentId":"grandchild","status":"running","parentId":"child","pid":42}]}"#;
    let out = forward_child_state_changed(line, &registry, "child")
        .expect("state_changed line should be forwarded");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["type"], "subagent_state_changed");
    let gc = find_subagent(&v, "grandchild");
    assert_eq!(
        gc["parentId"], "child",
        "descendant's real parent_id must be preserved so it nests under its true parent"
    );
    assert_eq!(gc["status"], "running");
    // Merged into the registry so polling (build_subagent_info_list) agrees.
    assert!(registry.lock().unwrap().contains_key("grandchild"));
}

#[test]
fn forward_child_state_changed_ignores_other_lines() {
    let registry = super::super::subagent_registry::new_registry();
    assert!(forward_child_state_changed(r#"{"type":"agent_end"}"#, &registry, "child").is_none());
    assert!(forward_child_state_changed("not json", &registry, "child").is_none());
}

#[test]
fn forward_child_state_changed_carries_union_not_partial_push() {
    // The root's own children must survive a descendant push: the forwarded
    // event carries the WHOLE registry (union), not a grandchildren-only list
    // that would evict siblings under the consumer's full-replace semantics
    // (#815 architecture review).
    let registry = super::super::subagent_registry::new_registry();
    registry
        .lock()
        .unwrap()
        .insert("child".to_string(), test_entry());
    let line = r#"{"type":"subagent_state_changed","subagents":[{"agentId":"grandchild","status":"running","parentId":"child"}]}"#;
    let out = forward_child_state_changed(line, &registry, "child").expect("forwarded");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let arr = v["subagents"].as_array().expect("subagents array");
    assert_eq!(
        arr.len(),
        2,
        "union must keep the root's child AND the grandchild"
    );
    assert_eq!(find_subagent(&v, "child")["agentId"], "child");
    assert_eq!(find_subagent(&v, "grandchild")["parentId"], "child");
}

#[test]
fn forward_child_state_changed_preserves_multiple_descendants() {
    // Two grandchildren under DIFFERENT parents in one event. A buggy impl that
    // stamps the whole list with a single id (or collapses it) would fail here;
    // each entry must keep its own agentId/parentId (#815).
    let registry = super::super::subagent_registry::new_registry();
    let line = r#"{"type":"subagent_state_changed","subagents":[{"agentId":"gc-a","status":"running","parentId":"child-1"},{"agentId":"gc-b","status":"idle","parentId":"child-2"}]}"#;
    let out = forward_child_state_changed(line, &registry, "child-1").expect("forwarded");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(find_subagent(&v, "gc-a")["parentId"], "child-1");
    assert_eq!(find_subagent(&v, "gc-a")["status"], "running");
    assert_eq!(find_subagent(&v, "gc-b")["parentId"], "child-2");
    assert_eq!(find_subagent(&v, "gc-b")["status"], "idle");
}

#[test]
fn forward_child_state_changed_chains_already_forwarded_descendants() {
    // Multi-hop: a great-grandchild's entry (parentId is itself a grandchild,
    // NOT the immediate child reporting it) arrives at this monitor. Forwarding
    // must NOT be gated on `parentId == agent_id`, or great-grandchildren stay
    // invisible — the original bug class (#815).
    let registry = super::super::subagent_registry::new_registry();
    let line = r#"{"type":"subagent_state_changed","subagents":[{"agentId":"great-grandchild","status":"running","parentId":"grandchild"}]}"#;
    let out = forward_child_state_changed(line, &registry, "grandchild")
        .expect("forwarded regardless of parentId");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(
        find_subagent(&v, "great-grandchild")["parentId"],
        "grandchild",
        "deep descendant identity must survive the hop unchanged"
    );
}

#[test]
fn forward_child_state_changed_caps_descendant_count() {
    // A misbehaving/compromised child cannot inject an unbounded descendant list
    // into the registry (#815 security review): the merge is capped.
    let registry = super::super::subagent_registry::new_registry();
    let entries: Vec<String> = (0..300)
        .map(|i| format!(r#"{{"agentId":"gc-{i}","status":"running","parentId":"child"}}"#))
        .collect();
    let line = format!(
        r#"{{"type":"subagent_state_changed","subagents":[{}]}}"#,
        entries.join(",")
    );
    forward_child_state_changed(&line, &registry, "child").expect("forwarded");
    assert_eq!(
        registry.lock().unwrap().len(),
        256,
        "merged descendant count must be capped at MAX_FORWARDED_SUBAGENTS"
    );
}

#[test]
fn handle_monitor_line_forwards_descendant_state_changed() {
    // Through the real handler: a child's subagent_state_changed (carrying a
    // grandchild) must be forwarded onto the parent/root broadcast so the TUI
    // panel can list it. Regression for #815: previously dropped by the
    // STATE_CHANGING_EVENTS pre-filter and never forwarded.
    let registry = super::super::subagent_registry::new_registry();
    registry
        .lock()
        .unwrap()
        .insert("child".to_string(), test_entry());
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(4);
    let line = r#"{"type":"subagent_state_changed","subagents":[{"agentId":"grandchild","status":"running","parentId":"child","pid":7}]}"#;
    super::handle_monitor_line(line, "child", &registry, None, Some(&tx), Some("root"));
    let fwd: serde_json::Value = serde_json::from_str(
        &rx.try_recv()
            .expect("descendant state_changed must be forwarded to the parent stream"),
    )
    .unwrap();
    assert_eq!(fwd["type"], "subagent_state_changed");
    // Union: both the root's child and the grandchild are present and correctly
    // attributed (the grandchild is NOT mis-shown under the wrong parent).
    assert_eq!(find_subagent(&fwd, "grandchild")["parentId"], "child");
    assert!(
        fwd["subagents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["agentId"] == "child")
    );
}

#[test]
fn forward_child_state_changed_preserves_existing_status_when_status_omitted() {
    let registry = super::super::subagent_registry::new_registry();
    {
        let mut guard = registry.lock().unwrap();
        guard.insert("child".to_string(), test_entry());
        let mut grandchild = test_entry();
        grandchild.status = SubagentStatus::Idle;
        grandchild.parent_id = Some("child".to_string());
        guard.insert("grandchild".to_string(), grandchild);
    }

    let line = r#"{"type":"subagent_state_changed","subagents":[{"agentId":"grandchild","parentId":"child"}]}"#;
    let out = forward_child_state_changed(line, &registry, "child").expect("forwarded");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();

    assert_eq!(
        registry.lock().unwrap()["grandchild"].status,
        SubagentStatus::Idle
    );
    assert_eq!(find_subagent(&v, "grandchild")["status"], "idle");
}

#[test]
fn handle_monitor_line_skips_full_state_broadcast_for_tool_boundaries() {
    let registry = super::super::subagent_registry::new_registry();
    let mut child = test_entry();
    child.status = SubagentStatus::Idle;
    registry.lock().unwrap().insert("child".to_string(), child);
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(4);

    super::handle_monitor_line(
        r#"{"type":"tool_execution_start","toolCallId":"c1","toolName":"bash","args":{}}"#,
        "child",
        &registry,
        None,
        Some(&tx),
        None,
    );
    assert!(
        rx.try_recv().is_err(),
        "tool start should not broadcast a full state snapshot; polling/notifications cover it"
    );

    super::handle_monitor_line(
        r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"bash","result":{"content":[]},"isError":false}"#,
        "child",
        &registry,
        None,
        Some(&tx),
        None,
    );
    assert!(
        rx.try_recv().is_err(),
        "successful tool end should not broadcast a redundant full state snapshot"
    );
}

#[test]
fn handle_monitor_line_tool_error_broadcasts_subagent_state_changed_promptly() {
    let registry = super::super::subagent_registry::new_registry();
    let mut child = test_entry();
    child.status = SubagentStatus::Running;
    registry.lock().unwrap().insert("child".to_string(), child);
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(4);

    super::handle_monitor_line(
        r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"bash","result":{"content":[]},"isError":true}"#,
        "child",
        &registry,
        None,
        Some(&tx),
        None,
    );

    let event: serde_json::Value = serde_json::from_str(
        &rx.try_recv()
            .expect("tool error should promptly broadcast subagent_state_changed"),
    )
    .unwrap();
    assert_eq!(event["type"], "subagent_state_changed");
    assert_eq!(find_subagent(&event, "child")["status"], "error");
}

#[test]
fn handle_monitor_line_agent_end_broadcasts_subagent_state_changed_promptly() {
    let registry = super::super::subagent_registry::new_registry();
    let mut child = test_entry();
    child.status = SubagentStatus::Running;
    registry.lock().unwrap().insert("child".to_string(), child);
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(4);

    super::handle_monitor_line(
        r#"{"type":"agent_end","messages":[]}"#,
        "child",
        &registry,
        None,
        Some(&tx),
        None,
    );

    let event: serde_json::Value = serde_json::from_str(
        &rx.try_recv()
            .expect("agent_end should promptly broadcast subagent_state_changed"),
    )
    .unwrap();
    assert_eq!(event["type"], "subagent_state_changed");
    assert_eq!(find_subagent(&event, "child")["status"], "idle");
}

#[test]
fn forward_child_state_changed_prunes_grandchild_absent_from_push() {
    // #831 nested case: a grandchild that was previously merged into the root
    // registry must be REMOVED when the forwarding child's next authoritative
    // push no longer lists it (it exited/was killed under the child). The
    // pure-upsert merge could never drop it, so it lingered in the TUI panel.
    let registry = super::super::subagent_registry::new_registry();
    {
        let mut g = registry.lock().unwrap();
        // root's direct child, plus a previously-merged grandchild under it.
        g.insert("child".to_string(), test_entry());
        let mut gc = test_entry();
        gc.parent_id = Some("child".to_string());
        g.insert("grandchild".to_string(), gc);
        // an unrelated sibling sub-tree that must survive the scoped prune.
        let mut sib = test_entry();
        sib.parent_id = Some("other".to_string());
        g.insert("sibling".to_string(), sib);
        g.insert("other".to_string(), test_entry());
    }
    // The child now reports an EMPTY descendant set (the grandchild is gone).
    let line = r#"{"type":"subagent_state_changed","subagents":[]}"#;
    let out = forward_child_state_changed(line, &registry, "child").expect("forwarded");
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    let ids: Vec<&str> = v["subagents"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["agentId"].as_str())
        .collect();
    // Grandchild pruned; the child itself and the unrelated tree are untouched.
    assert!(
        !ids.contains(&"grandchild"),
        "dead grandchild must be pruned"
    );
    assert!(ids.contains(&"child"));
    assert!(ids.contains(&"sibling"), "unrelated sub-tree must survive");
    assert!(ids.contains(&"other"));
    let g = registry.lock().unwrap();
    assert!(!g.contains_key("grandchild"));
    assert!(g.contains_key("sibling"));
}

// cascade_remove_and_state_changed tests moved to `subagent_cascade_tests.rs`
// alongside the extracted `subagent_cascade` module (#831).
