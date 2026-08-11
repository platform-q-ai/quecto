use crate::domain::ids::AgentUuid;
use crate::domain::session::{PersistedSubagentRosterEntry, SubagentLiveness};
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};
use crate::interface::cli::uds::uds_dispatch_session::{
    restore_persisted_subagent_roster, snapshot_subagent_roster, verify_persisted_live_subagent,
};

fn roster_entry(id: &str, socket_path: std::path::PathBuf) -> PersistedSubagentRosterEntry {
    PersistedSubagentRosterEntry {
        agent_uuid: id.to_string(),
        display_name: format!("worker-{id}"),
        session_key: id.to_string(),
        socket_path,
        pid: 1,
        liveness: SubagentLiveness::Live,
        parent_id: Some("parent".to_string()),
        read_only: true,
        status: Some("idle".to_string()),
    }
}

#[test]
fn snapshot_subagent_roster_serializes_sorted_liveness_metadata() {
    let registry = new_registry();
    {
        let mut entries = registry.lock().unwrap();
        let mut b = SubagentEntry::with_identity(
            AgentUuid::from("b".to_string()),
            "beta".to_string(),
            "/tmp/b.sock".into(),
            20,
        );
        b.persisted_liveness = SubagentLiveness::Dead;
        b.read_only = true;
        b.parent_id = Some("root".to_string());
        entries.insert("b".to_string(), b);
        let mut a = SubagentEntry::with_identity(
            AgentUuid::from("a".to_string()),
            "alpha".to_string(),
            "/tmp/a.sock".into(),
            10,
        );
        a.persisted_liveness = SubagentLiveness::Detached;
        entries.insert("a".to_string(), a);
    }

    let roster = snapshot_subagent_roster(&Some(registry));
    assert_eq!(roster.len(), 2);
    assert_eq!(roster[0].agent_uuid, "a");
    assert_eq!(roster[0].display_name, "alpha");
    assert_eq!(roster[0].liveness, SubagentLiveness::Detached);
    assert_eq!(roster[1].agent_uuid, "b");
    assert_eq!(roster[1].display_name, "beta");
    assert_eq!(roster[1].liveness, SubagentLiveness::Dead);
    assert_eq!(roster[1].parent_id.as_deref(), Some("root"));
    assert!(roster[1].read_only);
}

#[test]
fn verify_persisted_live_subagent_rejects_empty_uuid_or_socket() {
    assert!(!verify_persisted_live_subagent(&roster_entry(
        "",
        "/tmp/live.sock".into()
    )));
    assert!(!verify_persisted_live_subagent(&roster_entry(
        "child",
        "".into()
    )));
}

#[test]
fn verify_persisted_live_subagent_distinguishes_reachable_socket() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("child.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();

    assert!(verify_persisted_live_subagent(&roster_entry(
        "child",
        socket.clone()
    )));
    drop(listener);
    assert!(!verify_persisted_live_subagent(&roster_entry(
        "child", socket
    )));
}

#[test]
fn restore_persisted_roster_live_reachable_and_unreachable_liveness() {
    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("live.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();

    let mut live = roster_entry("live", socket);
    live.display_name = "Live worker".into();
    live.read_only = false;
    let mut unreachable = roster_entry("gone", dir.path().join("gone.sock"));
    unreachable.display_name = "Gone worker".into();
    let mut dead = roster_entry("dead", dir.path().join("dead.sock"));
    dead.liveness = SubagentLiveness::Dead;

    restore_persisted_subagent_roster(&Some(registry.clone()), vec![live, unreachable, dead]);

    let entries = registry.lock().unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(
        entries.get("live").unwrap().persisted_liveness,
        SubagentLiveness::Live
    );
    assert_eq!(entries.get("live").unwrap().status.to_wire_str(), "idle");
    assert_eq!(
        entries.get("gone").unwrap().persisted_liveness,
        SubagentLiveness::Detached
    );
    assert_eq!(entries.get("gone").unwrap().status.to_wire_str(), "exited");
    assert_eq!(
        entries.get("dead").unwrap().persisted_liveness,
        SubagentLiveness::Dead
    );
}

#[test]
fn restore_persisted_roster_no_registry_is_noop() {
    restore_persisted_subagent_roster(&None, vec![roster_entry("ignored", "".into())]);
}
