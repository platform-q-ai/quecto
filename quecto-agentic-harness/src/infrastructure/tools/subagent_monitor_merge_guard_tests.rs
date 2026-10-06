//! #2192 review round 5: what a child's report may not name. A report
//! naming the reporter's own parent under the reporter closes a cycle of
//! parents; walked without a visited set, the prune after the merge never
//! ended, under the registry lock (H1). A report may not name a row this
//! harness launched, by key or by label (M1), nor any ancestor of the
//! reporter, nor make a row its own ancestor.
use super::*;
use crate::domain::child_end::ChildOrigin;
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};

/// A registry holding one child this harness (`root-id`) launched:
/// `c-uuid`, labelled `worker`.
fn with_launched_child() -> SubagentRegistry {
    let registry = new_registry();
    let mut child = SubagentEntry::with_identity(
        crate::domain::ids::AgentUuid::new("c-uuid"),
        "worker".into(),
        "/tmp/c.sock".into(),
        4242,
    );
    child.origin = ChildOrigin::Launched;
    child.parent_id = Some("root-id".into());
    registry.lock().unwrap().insert("c-uuid".into(), child);
    registry
}

/// Merge `descendants` as reported by `reporter`, failing the whole test
/// binary — not hanging it, nor letting a runaway walk grow without end —
/// when the merge does not finish within a few seconds.
fn merge_within_deadline(
    registry: &SubagentRegistry,
    reporter: &str,
    descendants: serde_json::Value,
) {
    let (done, finished) = std::sync::mpsc::channel();
    let (registry, reporter) = (registry.clone(), reporter.to_string());
    std::thread::spawn(move || {
        let event = serde_json::json!({"type": "subagent_state_changed", "subagents": descendants});
        merge_and_forward_state_changed(&event, &registry, &reporter);
        let _ = done.send(());
    });
    if finished
        .recv_timeout(std::time::Duration::from_secs(10))
        .is_err()
    {
        eprintln!("the merge did not finish within 10 s: a walk of reported parents loops");
        std::process::exit(101);
    }
}

#[test]
fn a_child_reporting_its_own_parent_under_itself_does_not_loop_or_hold_the_lock() {
    let registry = with_launched_child();
    merge_within_deadline(
        &registry,
        "c-uuid",
        serde_json::json!([{"agentUuid": "root-id", "parentId": "c-uuid", "status": "running"}]),
    );
    let guard = registry
        .try_lock()
        .expect("the registry lock is free again");
    assert!(
        !guard.contains_key("root-id"),
        "the reporter's parent is refused"
    );
    assert_eq!(guard["c-uuid"].parent_id.as_deref(), Some("root-id"));
}

#[test]
fn reported_rows_cannot_make_a_cycle_of_parents() {
    let registry = with_launched_child();
    merge_within_deadline(
        &registry,
        "c-uuid",
        serde_json::json!([
            {"agentUuid": "x-uuid", "parentId": "y-uuid"},
            {"agentUuid": "y-uuid", "parentId": "x-uuid"},
            {"agentUuid": "c-uuid", "parentId": "x-uuid"}
        ]),
    );
    let guard = registry.lock().unwrap();
    assert_eq!(guard["x-uuid"].parent_id.as_deref(), Some("y-uuid"));
    assert!(!guard.contains_key("y-uuid"), "y under x would close x→y→x");
    assert_eq!(guard["c-uuid"].parent_id.as_deref(), Some("root-id"));
}

#[test]
fn a_walk_over_a_cycle_already_in_the_registry_ends() {
    let registry = new_registry();
    for (key, parent) in [("a", "b"), ("b", "a"), ("c", "b")] {
        let mut row = SubagentEntry::with_identity(
            crate::domain::ids::AgentUuid::new(key),
            key.into(),
            "/tmp/x.sock".into(),
            0,
        );
        row.parent_id = Some(parent.into());
        registry.lock().unwrap().insert(key.into(), row);
    }
    let guard = registry.lock().unwrap();
    let mut below_a = transitive_descendants(&guard, "a");
    below_a.sort();
    assert_eq!(below_a, ["b", "c"]);
    let ancestors = ancestry(&guard, "c");
    assert_eq!(ancestors.len(), 3, "{ancestors:?}");
}

#[test]
fn a_report_naming_a_launched_rows_label_or_an_ancestor_is_refused() {
    let registry = with_launched_child();
    merge_within_deadline(
        &registry,
        "c-uuid",
        serde_json::json!([
            {"agentUuid": "worker", "parentId": "c-uuid"},
            {"agentUuid": "g-uuid", "parentId": "c-uuid"}
        ]),
    );
    // A grandchild now reports its grandparent's parent under itself.
    merge_within_deadline(
        &registry,
        "c-uuid",
        serde_json::json!([
            {"agentUuid": "g-uuid", "parentId": "c-uuid"},
            {"agentUuid": "root-id", "parentId": "g-uuid"}
        ]),
    );
    let guard = registry.lock().unwrap();
    assert!(!guard.contains_key("worker"), "a launched row's label");
    assert!(
        !guard.contains_key("root-id"),
        "an ancestor of the reporter"
    );
    assert_eq!(guard["g-uuid"].origin, ChildOrigin::Reported);
}

/// #2192 review (PRRT_kwDORUxnPM6mhJd5): a reported descendant cannot take
/// a launched sibling's label (or key) as its display name: it is named by
/// its own key instead, and everything the parent is told of it names it
/// as reported, so it never reads as the launched child.
#[test]
fn a_reported_row_cannot_wear_a_launched_childs_name() {
    let registry = with_launched_child();
    merge_within_deadline(
        &registry,
        "c-uuid",
        serde_json::json!([
            {"agentUuid": "g-1", "displayName": "worker", "parentId": "c-uuid"},
            {"agentUuid": "g-2", "displayName": "c-uuid", "parentId": "c-uuid"},
            {"agentUuid": "g-3", "displayName": "helper", "parentId": "c-uuid"}
        ]),
    );
    let guard = registry.lock().unwrap();
    assert_eq!(guard["g-1"].display_name, "g-1", "not the sibling's label");
    assert_eq!(guard["g-2"].display_name, "g-2", "not the sibling's key");
    assert_eq!(
        guard["g-3"].display_name, "helper",
        "an unclaimed label is kept"
    );
    assert_eq!(guard["c-uuid"].display_name, "worker");
}

#[test]
fn every_parent_facing_label_of_a_reported_row_says_it_is_reported() {
    let registry = with_launched_child();
    merge_within_deadline(
        &registry,
        "c-uuid",
        serde_json::json!([{"agentUuid": "g-1", "displayName": "helper", "parentId": "c-uuid"}]),
    );
    let label = super::super::subagent_monitor::notification_display_label(&registry, "g-1");
    assert_eq!(label, "reported:g-1");
    let note = crate::infrastructure::tools::subagent_registry::SubagentNotification::Exited {
        agent_id: label,
        reason: Some("process_exit".into()),
        detail: Some("ended normally (exit code 0)".into()),
    }
    .to_message();
    assert!(note.starts_with("Sub-agent 'reported:g-1' "), "{note}");
    assert!(!note.contains("'worker'"), "{note}");
    assert_eq!(
        super::super::subagent_monitor::notification_display_label(&registry, "c-uuid"),
        "worker",
        "a launched child keeps its own label"
    );
    registry
        .lock()
        .unwrap()
        .get_mut("g-1")
        .unwrap()
        .persisted_liveness = crate::domain::sessions::entities::session::SubagentLiveness::Dead;
    let row = super::super::agent_cmd_ended::find_ended(&registry, "g-1").unwrap();
    assert_eq!(row.label, "reported:g-1");
}
