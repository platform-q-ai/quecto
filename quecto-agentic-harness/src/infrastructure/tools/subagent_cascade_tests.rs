// #831: cascade-remove of a dead sub-agent sub-tree + survivor-set broadcast.
use super::*;
use crate::infrastructure::tools::subagent_registry::new_registry;
use std::path::PathBuf;

fn child_entry(parent: &str) -> SubagentEntry {
    let mut e = SubagentEntry::new(PathBuf::from("/s"), 0);
    e.parent_id = Some(parent.to_string());
    e
}

fn seed_tree() -> SubagentRegistry {
    let r = new_registry();
    let mut g = r.lock().unwrap();
    g.insert("p".into(), SubagentEntry::new(PathBuf::from("/s"), 1));
    g.insert("c".into(), child_entry("p"));
    g.insert("gc".into(), child_entry("c"));
    g.insert("live".into(), SubagentEntry::new(PathBuf::from("/s"), 4));
    drop(g);
    r
}

#[test]
fn cascade_remove_drops_agent_and_transitive_descendants() {
    let r = new_registry();
    {
        let mut g = r.lock().unwrap();
        g.insert("p".into(), SubagentEntry::new(PathBuf::from("/s"), 1));
        g.insert("c".into(), child_entry("p"));
        g.insert("gc".into(), child_entry("c"));
        // An unrelated sibling tree that must survive.
        g.insert("other".into(), SubagentEntry::new(PathBuf::from("/s"), 2));
        g.insert("other-c".into(), child_entry("other"));
    }
    let mut removed_ids: Vec<String> = cascade_remove(&r, "p")
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    removed_ids.sort();
    assert_eq!(
        removed_ids,
        vec!["c".to_string(), "gc".to_string(), "p".to_string()]
    );
    let g = r.lock().unwrap();
    assert!(!g.contains_key("p"));
    assert!(!g.contains_key("c"));
    assert!(!g.contains_key("gc"));
    // Unrelated subtree is NEVER removed.
    assert!(g.contains_key("other"));
    assert!(g.contains_key("other-c"));
}

#[test]
fn cascade_remove_missing_agent_returns_empty() {
    let r = new_registry();
    r.lock()
        .unwrap()
        .insert("live".into(), SubagentEntry::new(PathBuf::from("/s"), 1));
    let removed = cascade_remove(&r, "ghost");
    assert!(removed.is_empty());
    assert!(r.lock().unwrap().contains_key("live"));
}

#[test]
fn cascade_remove_and_state_changed_emits_survivors_only() {
    let r = seed_tree();
    let event = cascade_remove_and_state_changed(&r, "p")
        .event
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
    assert!(
        cascade_remove_and_state_changed(&r, "ghost")
            .event
            .is_none()
    );
    assert_eq!(r.lock().unwrap().len(), 4);
}

#[test]
fn state_changed_event_serializes_read_only_observer_flag() {
    let r = new_registry();
    {
        let mut guard = r.lock().unwrap();
        let mut observer = SubagentEntry::new(PathBuf::from("/observer.sock"), 11);
        observer.read_only = true;
        guard.insert("observer".into(), observer);
        guard.insert(
            "worker".into(),
            SubagentEntry::new(PathBuf::from("/worker.sock"), 12),
        );
    }

    let event = build_state_changed_event(&r);
    let v: serde_json::Value = serde_json::from_str(&event).unwrap();

    let observer = v["subagents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["agentId"] == "observer")
        .expect("observer entry should be serialized");
    let worker = v["subagents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["agentId"] == "worker")
        .expect("worker entry should be serialized");

    assert_eq!(observer["readOnly"], true);
    assert_eq!(worker["readOnly"], false);
}

/// #1055: the canonical state_changed line must be `\n`-terminated so every
/// broadcast send site is correct by construction. A newline-less send made
/// the client writer splice the event onto the next line, so ancestor
/// monitors dropped BOTH events — new grandchildren never propagated and
/// exited ghosts were never pruned.
#[test]
fn state_changed_event_is_newline_terminated() {
    let r = new_registry();
    r.lock().unwrap().insert(
        "child".into(),
        SubagentEntry::new(PathBuf::from("/c.sock"), 1),
    );
    let event = build_state_changed_event(&r);
    assert!(
        event.ends_with('\n'),
        "broadcast line must end with newline"
    );
    assert!(
        !event[..event.len() - 1].contains('\n'),
        "exactly one terminating newline"
    );
}

/// #1055 framing regression: two consecutive broadcast events written
/// back-to-back (as the per-client writer does) must split into two
/// independently parseable JSON lines.
#[test]
fn consecutive_state_changed_events_stay_line_framed() {
    let r = new_registry();
    r.lock().unwrap().insert(
        "child".into(),
        SubagentEntry::new(PathBuf::from("/c.sock"), 1),
    );
    let wire = format!(
        "{}{}",
        build_state_changed_event(&r),
        build_state_changed_event(&r)
    );
    let lines: Vec<&str> = wire.lines().collect();
    assert_eq!(lines.len(), 2, "two sends must frame as two lines");
    for line in lines {
        serde_json::from_str::<serde_json::Value>(line).expect("each line parses independently");
    }
}
