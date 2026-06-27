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
    let out = forward_child_state_changed(line, &registry)
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
    assert!(forward_child_state_changed(r#"{"type":"agent_end"}"#, &registry).is_none());
    assert!(forward_child_state_changed("not json", &registry).is_none());
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
    let out = forward_child_state_changed(line, &registry).expect("forwarded");
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
    let out = forward_child_state_changed(line, &registry).expect("forwarded");
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
    let out =
        forward_child_state_changed(line, &registry).expect("forwarded regardless of parentId");
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
    forward_child_state_changed(&line, &registry).expect("forwarded");
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

// --- #831: cascade-remove on exit/kill broadcasts the survivor set ---

use super::super::subagent_registry::new_registry;

fn seed_tree() -> super::super::subagent_registry::SubagentRegistry {
    let r = new_registry();
    let mut g = r.lock().unwrap();
    g.insert("p".into(), SubagentEntry::new(PathBuf::from("/s"), 1));
    let mut c = SubagentEntry::new(PathBuf::from("/s"), 2);
    c.parent_id = Some("p".into());
    g.insert("c".into(), c);
    let mut gc = SubagentEntry::new(PathBuf::from("/s"), 3);
    gc.parent_id = Some("c".into());
    g.insert("gc".into(), gc);
    g.insert("live".into(), SubagentEntry::new(PathBuf::from("/s"), 4));
    drop(g);
    r
}

#[test]
fn cascade_remove_and_state_changed_emits_survivors_only() {
    let r = seed_tree();
    let event = cascade_remove_and_state_changed(&r, "p")
        .expect("removing a live subtree must yield a broadcast event");
    let v: serde_json::Value = serde_json::from_str(&event).unwrap();
    assert_eq!(v["type"], "subagent_state_changed");
    let ids: Vec<&str> = v["subagents"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["agentId"].as_str())
        .collect();
    // The whole dead subtree is gone; the unrelated live agent remains.
    assert_eq!(ids, vec!["live"], "broadcast must list only survivors");
    let g = r.lock().unwrap();
    assert!(!g.contains_key("p") && !g.contains_key("c") && !g.contains_key("gc"));
    assert!(g.contains_key("live"));
}

#[test]
fn cascade_remove_and_state_changed_noop_returns_none() {
    let r = seed_tree();
    // Nothing to remove -> no broadcast (avoid redundant churn).
    assert!(cascade_remove_and_state_changed(&r, "ghost").is_none());
    assert_eq!(r.lock().unwrap().len(), 4);
}
