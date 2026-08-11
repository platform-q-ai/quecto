use crate::domain::session::SubagentLiveness;

use super::subagent_registry::{SubagentEntry, lookup_subagent_socket, new_registry};
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

#[test]
fn rejects_non_live_liveness_for_command_and_inspection_routes() {
    let reg = new_registry();
    let mut detached = entry("/tmp/detached.sock", None);
    detached.display_name = "detached".into();
    detached.persisted_liveness = SubagentLiveness::Detached;
    let mut dead = entry("/tmp/dead.sock", None);
    dead.display_name = "dead".into();
    dead.persisted_liveness = SubagentLiveness::Dead;
    let mut guard = reg.lock().unwrap();
    guard.insert("detached".into(), detached);
    guard.insert("dead".into(), dead);
    drop(guard);

    assert!(
        lookup_subagent_socket(&reg, "detached")
            .unwrap_err()
            .contains("detached")
    );
    assert!(
        lookup_subagent_socket(&reg, "dead")
            .unwrap_err()
            .contains("dead")
    );
    assert!(
        resolve_inspection_route(&reg, "detached")
            .unwrap_err()
            .contains("detached")
    );
    assert!(
        resolve_inspection_route(&reg, "dead")
            .unwrap_err()
            .contains("dead")
    );
}

#[test]
fn routable_command_mapping_covers_uds_allowlist_and_rejects_mutating_commands() {
    assert_eq!(
        RoutableInspectionCommand::from_uds_type("get_messages"),
        Some(RoutableInspectionCommand::GetMessages)
    );
    assert_eq!(
        RoutableInspectionCommand::from_uds_type("get_messages_tail"),
        Some(RoutableInspectionCommand::GetMessagesTail)
    );
    assert_eq!(
        RoutableInspectionCommand::from_uds_type("get_message"),
        Some(RoutableInspectionCommand::GetMessage)
    );
    assert_eq!(
        RoutableInspectionCommand::from_uds_type("sync"),
        Some(RoutableInspectionCommand::Sync)
    );
    assert_eq!(
        RoutableInspectionCommand::from_uds_type("get_state"),
        Some(RoutableInspectionCommand::GetState)
    );
    assert_eq!(RoutableInspectionCommand::from_uds_type("prompt"), None);
    assert_eq!(RoutableInspectionCommand::from_uds_type("abort"), None);
}
