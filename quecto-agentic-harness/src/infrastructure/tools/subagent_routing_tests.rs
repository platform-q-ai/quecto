use super::subagent_registry::{SubagentEntry, new_registry};
use super::subagent_routing::{
    InspectionRoute, RoutableInspectionCommand, resolve_inspection_route,
};
use std::path::PathBuf;

fn entry(sock: &str, parent: Option<&str>) -> SubagentEntry {
    let mut e = SubagentEntry::new(PathBuf::from(sock), 1);
    e.display_name = sock.trim_start_matches('/').to_string();
    e.parent_id = parent.map(str::to_string);
    e
}

#[test]
fn resolves_direct_before_ancestor_lookup() {
    let reg = new_registry();
    reg.lock()
        .unwrap()
        .insert("child".into(), entry("/tmp/child.sock", Some("parent")));
    assert!(matches!(
        resolve_inspection_route(&reg, "child").unwrap(),
        InspectionRoute::Direct { .. }
    ));
}

#[test]
fn resolves_via_nearest_connectable_ancestor() {
    let reg = new_registry();
    let mut parent = entry("/tmp/parent.sock", None);
    parent.display_name = "parent".into();
    let mut child = entry("", Some("parent"));
    child.display_name = "child".into();
    reg.lock().unwrap().insert("parent".into(), parent);
    reg.lock().unwrap().insert("child".into(), child);
    match resolve_inspection_route(&reg, "child").unwrap() {
        InspectionRoute::ViaAncestor {
            ancestor_id,
            target_id,
            ..
        } => {
            assert_eq!(ancestor_id, "parent");
            assert_eq!(target_id, "child");
        }
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn rejects_cycles_and_excess_depth() {
    let reg = new_registry();
    let mut a = entry("", Some("b"));
    a.display_name = "a".into();
    let mut b = entry("", Some("a"));
    b.display_name = "b".into();
    reg.lock().unwrap().insert("a".into(), a);
    reg.lock().unwrap().insert("b".into(), b);
    assert!(
        resolve_inspection_route(&reg, "a")
            .unwrap_err()
            .contains("cycle")
    );
}

#[test]
fn rejects_missing_ancestor_with_non_connectable_error() {
    let reg = new_registry();
    let mut child = entry("", None);
    child.display_name = "child".into();
    reg.lock().unwrap().insert("child".into(), child);
    assert!(
        resolve_inspection_route(&reg, "child")
            .unwrap_err()
            .contains("no ancestor-connectable socket")
    );
}

#[test]
fn exact_parent_key_wins_over_duplicate_display_name_aliases() {
    let reg = new_registry();
    let mut parent = entry("/tmp/parent.sock", None);
    parent.display_name = "builder".into();
    let mut duplicate = entry("/tmp/duplicate.sock", None);
    duplicate.display_name = "builder".into();
    let mut child = entry("", Some("parent-key"));
    child.display_name = "child".into();
    let mut guard = reg.lock().unwrap();
    guard.insert("parent-key".into(), parent);
    guard.insert("duplicate-key".into(), duplicate);
    guard.insert("child".into(), child);
    drop(guard);

    match resolve_inspection_route(&reg, "child").unwrap() {
        InspectionRoute::ViaAncestor { target_id, .. } => assert_eq!(target_id, "child"),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn agent_cmd_allowlist_is_closed() {
    assert_eq!(
        RoutableInspectionCommand::from_agent_cmd("get_messages"),
        Some(RoutableInspectionCommand::GetMessages)
    );
    assert_eq!(
        RoutableInspectionCommand::from_agent_cmd("get_state"),
        Some(RoutableInspectionCommand::GetState)
    );
    assert_eq!(RoutableInspectionCommand::from_agent_cmd("prompt"), None);
}
