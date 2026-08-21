use super::*;
use crate::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentStatus, WorkflowSnapshot, new_registry,
};

fn add(
    registry: &crate::infrastructure::tools::subagent_registry::SubagentRegistry,
    id: &str,
    parent: Option<&str>,
) {
    let mut entry = SubagentEntry::new(format!("/tmp/{id}.sock").into(), id.len() as u32);
    entry.parent_id = parent.map(str::to_owned);
    registry.lock().unwrap().insert(id.to_string(), entry);
}

#[test]
fn cascade_remove_removes_transitive_tree_only_and_reports_unknown_empty() {
    let registry = new_registry();
    add(&registry, "root", None);
    add(&registry, "child", Some("root"));
    add(&registry, "grand", Some("child"));
    add(&registry, "sibling", None);

    let removed = cascade_remove(&registry, "child");
    let mut ids: Vec<_> = removed.into_iter().map(|(id, _)| id).collect();
    ids.sort();
    assert_eq!(ids, vec!["child", "grand"]);
    {
        let guard = registry.lock().unwrap();
        assert!(guard.contains_key("root"));
        assert!(guard.contains_key("sibling"));
    }
    assert!(cascade_remove(&registry, "missing").is_empty());
}

#[test]
fn build_state_changed_event_serializes_sorted_known_fields_with_newline() {
    let registry = new_registry();
    add(&registry, "b", None);
    add(&registry, "a", Some("b"));
    {
        let mut guard = registry.lock().unwrap();
        let a = guard.get_mut("a").unwrap();
        a.status = SubagentStatus::Running;
        a.last_tool = Some("bash".into());
        a.last_error = Some("boom".into());
        a.read_only = true;
        a.workflow = Some(WorkflowSnapshot {
            mode: "active".into(),
            steps_completed: 1,
            steps_total: 2,
        });
    }
    let line = build_state_changed_event(&registry);
    assert!(line.ends_with('\n'));
    let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
    assert_eq!(v["type"], "subagent_state_changed");
    let arr = v["subagents"].as_array().unwrap();
    assert_eq!(arr[0]["agentId"], "a");
    assert_eq!(arr[0]["parentId"], "b");
    assert_eq!(arr[0]["workflow"]["mode"], "active");
}

#[test]
fn cascade_remove_and_state_changed_returns_survivor_event_only_when_removed() {
    let registry = new_registry();
    add(&registry, "dead", None);
    add(&registry, "live", None);
    let out = cascade_remove_and_state_changed(&registry, "dead");
    assert_eq!(out.removed.len(), 1);
    let event = out.event.unwrap();
    assert!(event.contains("live"));
    assert!(!event.contains("dead"));
    let none = cascade_remove_and_state_changed(&registry, "dead");
    assert_eq!(none.removed.len(), 1);
    assert!(none.event.is_some());
    assert_eq!(
        registry.lock().unwrap()["dead"].status,
        SubagentStatus::Exited
    );
}

fn poison(registry: &crate::infrastructure::tools::subagent_registry::SubagentRegistry) {
    let r = registry.clone();
    let _ = std::thread::spawn(move || {
        let _guard = r.lock().unwrap();
        panic!("poison the registry lock");
    })
    .join();
}

#[test]
fn cascade_helpers_recover_from_poisoned_registry_lock() {
    let registry = new_registry();
    add(&registry, "root", None);
    add(&registry, "kid", Some("root"));
    poison(&registry);

    // All three public helpers must recover via into_inner() and keep working.
    let event = build_state_changed_event(&registry);
    assert!(event.ends_with('\n') && event.contains("root"));

    let removed = cascade_remove(&registry, "kid");
    assert_eq!(removed.len(), 1);

    let out = cascade_remove_and_state_changed(&registry, "root");
    assert_eq!(out.removed.len(), 2);
    assert!(out.event.unwrap().contains("subagent_state_changed"));
}
