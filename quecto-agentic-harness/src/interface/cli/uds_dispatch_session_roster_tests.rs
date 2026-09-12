//! Session roster persistence and restore (#1937, epic #1929).
//!
//! Restore keeps history and creates no operational child row: a
//! launcher-created child cannot outlive its launcher, so persisted rows are
//! read only to be ignored. No socket probe, no pid compare, no readoption.
use crate::domain::ids::AgentUuid;
use crate::domain::session::{
    PersistedSubagentRosterEntry, SubagentLiveness, SubagentRestoreReason,
};
use crate::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentStatus, new_registry,
};
use crate::interface::cli::uds::uds_dispatch_session::{
    reset_subagent_roster_on_restore, snapshot_subagent_roster,
    snapshot_subagent_roster_with_restore_reason,
};

fn roster_entry(id: &str) -> PersistedSubagentRosterEntry {
    PersistedSubagentRosterEntry {
        agent_uuid: id.to_string(),
        display_name: format!("worker-{id}"),
        session_key: id.to_string(),
        liveness: SubagentLiveness::Live,
        restore_reason: SubagentRestoreReason::LegacyUnspecified,
        parent_id: Some("parent".to_string()),
        read_only: true,
        delivered_message_ordinal: None,
        pending_message_reports: std::collections::VecDeque::new(),
        status: Some("idle".to_string()),
    }
}

/// A row exactly as a pre-#1937 harness wrote it: with the child's socket
/// path and pid as recovery authority.
fn legacy_row(
    id: &str,
    socket_path: &std::path::Path,
    liveness: &str,
    status: &str,
) -> PersistedSubagentRosterEntry {
    serde_json::from_value(serde_json::json!({
        "agentUuid": id,
        "displayName": format!("worker-{id}"),
        "sessionKey": id,
        "socketPath": socket_path,
        "pid": 4242,
        "liveness": liveness,
        "status": status,
        "parentId": "parent",
        "readOnly": true
    }))
    .unwrap()
}

/// A listener that must never be connected to: `accept` is non-blocking so
/// a probe would be observed as an accepted connection, and its absence as
/// `WouldBlock`.
fn listener_that_must_stay_silent(path: &std::path::Path) -> std::os::unix::net::UnixListener {
    let listener = std::os::unix::net::UnixListener::bind(path).unwrap();
    listener.set_nonblocking(true).unwrap();
    listener
}

fn assert_never_connected(listener: &std::os::unix::net::UnixListener, what: &str) {
    match listener.accept() {
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
        other => panic!("{what}: restore must never probe a persisted socket, got {other:?}"),
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

/// #1937: the child's pid and socket are no longer written as recovery
/// authority. A row carries history only.
#[test]
fn snapshot_never_writes_child_pid_or_socket() {
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "w".into(),
        SubagentEntry::with_identity(
            AgentUuid::from("w".to_string()),
            "worker".to_string(),
            "/tmp/w.sock".into(),
            4242,
        ),
    );
    let roster = snapshot_subagent_roster(&Some(registry));
    let json = serde_json::to_value(&roster).unwrap();
    let row = &json[0];
    assert!(row.get("pid").is_none(), "pid written: {row}");
    assert!(row.get("socketPath").is_none(), "socket written: {row}");
    assert!(!json.to_string().contains("/tmp/w.sock"));
    assert!(!json.to_string().contains("4242"));
}

#[test]
fn ordinary_exit_snapshot_persists_no_operational_children() {
    let registry = new_registry();
    {
        let mut entries = registry.lock().unwrap();
        let mut live = SubagentEntry::with_identity(
            AgentUuid::from("live"),
            "Live worker".into(),
            "/tmp/live.sock".into(),
            1,
        );
        live.status = SubagentStatus::Idle;
        live.persisted_liveness = SubagentLiveness::Live;
        entries.insert("live".into(), live);
    }

    let roster = snapshot_subagent_roster_with_restore_reason(
        &Some(registry),
        SubagentRestoreReason::OrdinaryTuiExitStopped,
    );
    assert!(
        roster.is_empty(),
        "killing exit persists no operational children"
    );
}

/// The legacy reader ignores `socketPath` and `pid` (they are not part of
/// the record any more) and tolerates unknown reasons and missing identity.
#[test]
fn legacy_reader_ignores_pid_and_socket_and_tolerates_malformed_rows() {
    let legacy = legacy_row("old", std::path::Path::new("/tmp/old.sock"), "live", "idle");
    assert_eq!(legacy.agent_uuid, "old");
    assert_eq!(legacy.liveness, SubagentLiveness::Live);
    let rewritten = serde_json::to_value(&legacy).unwrap();
    assert!(rewritten.get("socketPath").is_none(), "{rewritten}");
    assert!(rewritten.get("pid").is_none(), "{rewritten}");

    let unknown: PersistedSubagentRosterEntry = serde_json::from_value(serde_json::json!({
        "agentUuid": "future",
        "restoreReason": "future_reason"
    }))
    .unwrap();
    assert_eq!(unknown.restore_reason, SubagentRestoreReason::Unknown);

    let malformed: PersistedSubagentRosterEntry = serde_json::from_value(serde_json::json!({
        "displayName": "missing identity"
    }))
    .unwrap();
    assert!(malformed.agent_uuid.is_empty());
    assert_eq!(malformed.liveness, SubagentLiveness::Dead);
    assert_eq!(
        malformed.restore_reason,
        SubagentRestoreReason::LegacyUnspecified
    );
}

/// #1937: no persisted row of any kind — live, detached, dead, explicitly
/// killed, unknown-reason or malformed — becomes an operational child, and
/// the sockets they name are never probed.
#[test]
fn restore_creates_no_operational_row_and_probes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let live_socket = dir.path().join("live.sock");
    let detached_socket = dir.path().join("detached.sock");
    let live_listener = listener_that_must_stay_silent(&live_socket);
    let detached_listener = listener_that_must_stay_silent(&detached_socket);

    let mut killed = roster_entry("killed");
    killed.restore_reason = SubagentRestoreReason::ExplicitlyKilled;
    let mut stopped = roster_entry("stopped");
    stopped.restore_reason = SubagentRestoreReason::OrdinaryTuiExitStopped;
    let mut unknown = roster_entry("unknown");
    unknown.restore_reason = SubagentRestoreReason::Unknown;
    let malformed: PersistedSubagentRosterEntry =
        serde_json::from_value(serde_json::json!({"displayName": "no identity"})).unwrap();
    let rows = vec![
        legacy_row("live", &live_socket, "live", "idle"),
        legacy_row("detached", &detached_socket, "detached", "idle"),
        legacy_row("dead", &dir.path().join("dead.sock"), "dead", "exited"),
        legacy_row("gone", &dir.path().join("gone.sock"), "live", "running"),
        killed,
        stopped,
        unknown,
        malformed,
    ];

    let registry = new_registry();
    reset_subagent_roster_on_restore(&Some(registry.clone()), &rows);

    assert_never_connected(&live_listener, "live row");
    assert_never_connected(&detached_listener, "detached row");
    assert!(
        registry.lock().unwrap().is_empty(),
        "restore creates no operational child row: {:?}",
        registry.lock().unwrap().keys().collect::<Vec<_>>()
    );
    assert!(
        crate::interface::cli::protocol::build_compact_subagent_roster(
            &Some(registry.clone()),
            None
        )
        .unwrap()
        .subagents
        .is_empty()
    );
    assert!(
        crate::interface::cli::protocol::build_live_subagent_info_list(&Some(registry)).is_empty()
    );
}

/// A legacy row can neither be sent to nor killed after restore: it is not
/// there. The stale socket it named stays untouched.
#[test]
fn legacy_rows_are_not_sendable_or_running_after_restore() {
    use crate::domain::tool::Tool;
    use crate::infrastructure::tools::agent_cmd::AgentCmdTool;

    let dir = tempfile::tempdir().unwrap();
    let stale_socket = dir.path().join("stale.sock");
    let listener = listener_that_must_stay_silent(&stale_socket);
    let registry = new_registry();
    reset_subagent_roster_on_restore(
        &Some(registry.clone()),
        &[legacy_row("restored", &stale_socket, "live", "running")],
    );

    let lookup = crate::infrastructure::tools::subagent_registry::lookup_subagent_socket(
        &registry, "restored",
    );
    assert!(lookup.is_err(), "a legacy row must not expose a socket");

    let rt = tokio::runtime::Runtime::new().unwrap();
    for agent_ref in ["worker-restored", "restored"] {
        for command in ["kill", "get_state"] {
            let result = rt
                .block_on(AgentCmdTool::new(registry.clone()).execute(&format!(
                    r#"{{"agent_id":"{agent_ref}","command":"{command}"}}"#
                )))
                .unwrap();
            assert!(
                result.is_error,
                "legacy row must not be targetable by {agent_ref} {command}"
            );
        }
        let send = rt
            .block_on(AgentCmdTool::new(registry.clone()).execute(&format!(
                r#"{{"agent_id":"{agent_ref}","command":"send","message":"hello"}}"#
            )))
            .unwrap();
        assert!(send.is_error, "legacy row must not be sendable");
    }
    assert!(registry.lock().unwrap().is_empty());
    assert_never_connected(&listener, "stale socket");
}

/// Restore replaces whatever operational rows the registry held, and the
/// worker the master then re-spawns has a fresh identity, not the legacy one.
#[test]
fn restore_clears_the_registry_and_a_respawn_gets_a_fresh_identity() {
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "stale".into(),
        SubagentEntry::new("/tmp/stale.sock".into(), 0),
    );
    let legacy = roster_entry("legacy-uuid");
    reset_subagent_roster_on_restore(&Some(registry.clone()), &[legacy]);
    assert!(registry.lock().unwrap().is_empty());

    // The explicit re-spawn registers through the normal path with a
    // minted uuid and its own launch generation; nothing about the legacy
    // row seeds it.
    let respawned = SubagentEntry::new("/tmp/respawned.sock".into(), 0);
    assert_ne!(respawned.agent_uuid.as_str(), "legacy-uuid");
    let earlier = crate::infrastructure::processes::parent_control::mint_credential();
    let later = crate::infrastructure::processes::parent_control::mint_credential();
    assert!(later.generation > earlier.generation);
    registry
        .lock()
        .unwrap()
        .insert(respawned.agent_uuid.as_str().to_string(), respawned.clone());
    let entries = registry.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(!entries.contains_key("legacy-uuid"));
    assert!(entries.contains_key(respawned.agent_uuid.as_str()));
}

#[test]
fn restore_with_no_registry_is_a_noop() {
    reset_subagent_roster_on_restore(&None, &[roster_entry("ignored")]);
}

#[test]
fn ordinary_exit_resume_cycles_stay_empty_of_old_children() {
    use crate::interface::cli::protocol::build_compact_subagent_roster;

    for already_dead in [false, true] {
        let registry = new_registry();
        let mut child = SubagentEntry::with_identity(
            AgentUuid::from("exit-child"),
            "Exit worker".into(),
            "/tmp/exit-child.sock".into(),
            42,
        );
        child.parent_id = Some("parent".into());
        child.status = if already_dead {
            SubagentStatus::Exited
        } else {
            SubagentStatus::Idle
        };
        child.persisted_liveness = if already_dead {
            SubagentLiveness::Dead
        } else {
            SubagentLiveness::Live
        };
        registry.lock().unwrap().insert("exit-child".into(), child);
        let snapshot = snapshot_subagent_roster_with_restore_reason(
            &Some(registry),
            SubagentRestoreReason::OrdinaryTuiExitStopped,
        );
        assert!(snapshot.is_empty());
        let restored = new_registry();
        reset_subagent_roster_on_restore(&Some(restored.clone()), &snapshot);
        for _ in 0..3 {
            let roster = build_compact_subagent_roster(&Some(restored.clone()), None).unwrap();
            assert!(roster.subagents.is_empty());
            let next = snapshot_subagent_roster_with_restore_reason(
                &Some(restored.clone()),
                SubagentRestoreReason::OrdinaryTuiExitStopped,
            );
            reset_subagent_roster_on_restore(&Some(restored.clone()), &next);
        }
        assert!(restored.lock().unwrap().is_empty());
    }
}

#[test]
fn snapshot_and_restore_recover_from_a_poisoned_registry_lock() {
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "worker".into(),
        SubagentEntry::new("/tmp/worker.sock".into(), 0),
    );
    let shared = registry.clone();
    let _ = std::thread::spawn(move || {
        let _guard = shared.lock().unwrap();
        panic!("poison registry for coverage");
    })
    .join();
    assert!(registry.lock().is_err());
    let roster = snapshot_subagent_roster_with_restore_reason(
        &Some(registry.clone()),
        SubagentRestoreReason::LegacyUnspecified,
    );
    assert_eq!(roster.len(), 1);
    reset_subagent_roster_on_restore(&Some(registry.clone()), &roster);
    assert!(
        registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    );
}
