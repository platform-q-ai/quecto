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
    note_persisted_roster_is_history, reset_subagent_roster, snapshot_subagent_roster,
    snapshot_subagent_roster_with_restore_reason,
};

/// What a resume does with the departing roster once its children have
/// settled (#1938): note the persisted rows as history, replace the records.
fn reset_roster_for_restore(
    registry: &Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    persisted: &[PersistedSubagentRosterEntry],
) {
    note_persisted_roster_is_history(registry, persisted);
    reset_subagent_roster(registry, "resume_session").expect("no live delegated row remains");
}

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

/// A session file exactly as a pre-#1937 harness wrote it: the roster rows
/// carry the children's socket paths and pids as recovery authority. Only
/// this on-disk shape still names a socket — the in-memory row type does
/// not — so the "never probed" assertion is made on what the real store
/// loads from it.
async fn legacy_session_on_disk(
    dir: &std::path::Path,
    key: &str,
    rows: serde_json::Value,
) -> crate::domain::session::Session {
    use crate::application::sessions::ports::SessionStore;
    let store = crate::infrastructure::persistence::session_store::FileSessionStore::new(
        crate::infrastructure::persistence::session_layout::FlatSessionLayout::new(dir),
    );
    let path = dir.join("sessions").join(format!(
        "{}.json",
        crate::infrastructure::persistence::filename::sanitize_session_key(key)
    ));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let snapshot = serde_json::json!({
        "type": "snapshot",
        "key": key,
        "messages": [{"role":"user","content":"history"}],
        "workflow_run": null,
        "subagent_roster": rows,
    });
    std::fs::write(&path, format!("{snapshot}\n")).unwrap();
    store
        .load(&crate::domain::session_identity::SessionIdentity::from_persisted_key(key))
        .await
        .unwrap()
        .expect("the store loads the legacy file")
}

fn legacy_row_json(
    id: &str,
    socket: &std::path::Path,
    liveness: &str,
    status: &str,
) -> serde_json::Value {
    serde_json::json!({
        "agentUuid": id,
        "displayName": format!("worker-{id}"),
        "sessionKey": id,
        "socketPath": socket,
        "pid": std::process::id(),
        "liveness": liveness,
        "status": status,
        "parentId": "parent",
        "readOnly": true
    })
}

/// #1937: no persisted row of any kind — live, detached, dead, explicitly
/// killed, unknown-reason or malformed — becomes an operational child, and
/// the sockets the legacy file names are never probed. The rows come from a
/// real legacy session file through the real store, the only path on which
/// a socket string still exists.
#[tokio::test]
async fn restore_creates_no_operational_row_and_probes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let live_socket = dir.path().join("live.sock");
    let detached_socket = dir.path().join("detached.sock");
    let live_listener = listener_that_must_stay_silent(&live_socket);
    let detached_listener = listener_that_must_stay_silent(&detached_socket);
    let raw = serde_json::to_string(&serde_json::json!([
        legacy_row_json("live", &live_socket, "live", "idle"),
        legacy_row_json("detached", &detached_socket, "detached", "idle"),
        legacy_row_json("dead", &dir.path().join("dead.sock"), "dead", "exited"),
        legacy_row_json("gone", &dir.path().join("gone.sock"), "live", "running"),
        {"agentUuid": "killed", "restoreReason": "explicitly_killed", "liveness": "live"},
        {"agentUuid": "stopped", "restoreReason": "ordinary_tui_exit_stopped", "liveness": "live"},
        {"agentUuid": "unknown", "restoreReason": "future_reason", "liveness": "live"},
        {"displayName": "no identity"},
    ]))
    .unwrap();
    assert!(raw.contains(&live_socket.display().to_string()));
    let loaded = legacy_session_on_disk(
        dir.path(),
        "cli:legacy",
        serde_json::from_str(&raw).unwrap(),
    )
    .await;
    assert_eq!(
        loaded.subagent_roster.len(),
        8,
        "every row is read, none dropped"
    );
    assert!(
        loaded
            .subagent_roster
            .iter()
            .any(|row| row.liveness == SubagentLiveness::Live),
        "a live legacy row is exactly what readoption would have probed"
    );

    let registry = new_registry();
    reset_roster_for_restore(&Some(registry.clone()), &loaded.subagent_roster);

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
    use crate::application::tools::ports::Tool;
    use crate::infrastructure::tools::agent_cmd::AgentCmdTool;

    let dir = tempfile::tempdir().unwrap();
    let stale_socket = dir.path().join("stale.sock");
    let listener = listener_that_must_stay_silent(&stale_socket);
    let registry = new_registry();
    reset_roster_for_restore(
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
    reset_roster_for_restore(&Some(registry.clone()), &[legacy]);
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
    reset_roster_for_restore(&None, &[roster_entry("ignored")]);
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
        reset_roster_for_restore(&Some(restored.clone()), &snapshot);
        for _ in 0..3 {
            let roster = build_compact_subagent_roster(&Some(restored.clone()), None).unwrap();
            assert!(roster.subagents.is_empty());
            let next = snapshot_subagent_roster_with_restore_reason(
                &Some(restored.clone()),
                SubagentRestoreReason::OrdinaryTuiExitStopped,
            );
            reset_roster_for_restore(&Some(restored.clone()), &next);
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
    reset_roster_for_restore(&Some(registry.clone()), &roster);
    assert!(
        registry
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    );
}

/// The composed sessions handles over `base` (#1970), built here so the
/// shared dispatch test env never names the composition layer itself.
pub(crate) fn composed_sessions(
    base: &std::path::Path,
) -> crate::interface::cli::uds_session_handles::SessionHandles {
    composed_sessions_for(base, "cli:test", None)
}

/// The composed sessions handles over `base` opened on `session_key`, with
/// the agent's retention store `spill_store` paired to the active session.
pub(crate) fn composed_sessions_for(
    base: &std::path::Path,
    session_key: &str,
    spill_store: Option<std::sync::Arc<dyn crate::application::sessions::ports::ContextSpillStore>>,
) -> crate::interface::cli::uds_session_handles::SessionHandles {
    crate::composition::sessions::build_session_handles(
        crate::interface::cli::uds_session_handles::SessionLoopInputs {
            base_dir: base.to_path_buf(),
            store: None,
            session_key: session_key.to_string(),
            spill_store,
        },
    )
}

/// The read handles of a session opened on `session_key` over the rig's
/// own `store`, with `messages` already published as its live transcript.
pub(crate) fn read_handles_over(
    store: std::sync::Arc<dyn crate::application::sessions::ports::SessionStore>,
    session_key: &str,
    spill_store: Option<std::sync::Arc<dyn crate::application::sessions::ports::ContextSpillStore>>,
    messages: &[crate::domain::message::Message],
) -> crate::interface::cli::uds_session_handles::SessionReadHandles {
    let handles = crate::composition::sessions::build_session_handles(
        crate::interface::cli::uds_session_handles::SessionLoopInputs {
            base_dir: std::path::PathBuf::new(),
            store: Some(store),
            session_key: session_key.to_string(),
            spill_store,
        },
    )
    .read_handles();
    let _ = handles
        .active_session
        .try_write()
        .expect("fresh session is uncontended")
        .publish(messages);
    handles
}

/// The read handles of a session opened on `session_key` over `base`, with
/// `messages` already published as its live transcript.
pub(crate) fn seeded_read_handles(
    base: &std::path::Path,
    session_key: &str,
    spill_store: Option<std::sync::Arc<dyn crate::application::sessions::ports::ContextSpillStore>>,
    messages: &[crate::domain::message::Message],
) -> crate::interface::cli::uds_session_handles::SessionReadHandles {
    let handles = composed_sessions_for(base, session_key, spill_store).read_handles();
    let _ = handles
        .active_session
        .try_write()
        .expect("fresh session is uncontended")
        .publish(messages);
    handles
}

/// Read handles of a session opened on `session_key` over a throwaway base
/// directory (leaked for the test's lifetime), with `messages` published.
pub(crate) fn read_handles_for(
    session_key: &str,
    spill_store: Option<std::sync::Arc<dyn crate::application::sessions::ports::ContextSpillStore>>,
    messages: &[crate::domain::message::Message],
) -> crate::interface::cli::uds_session_handles::SessionReadHandles {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let base = tmp.path().to_path_buf();
    std::mem::forget(tmp);
    seeded_read_handles(&base, session_key, spill_store, messages)
}

/// The full message a stable ref resolves to through the composed recovery
/// owner (ledger, live view, retention store), without a fallback slice.
pub(crate) async fn resolve_message(
    handles: &crate::interface::cli::uds_session_handles::SessionReadHandles,
    message_id: &str,
) -> Option<crate::domain::message::Message> {
    use crate::application::sessions::dto::RecoveredContent;
    use crate::interface::uds::sessions::recover_message_controller::GetMessageFields;
    match handles
        .recover_message
        .recover(
            GetMessageFields {
                message_id,
                tool_call_id: None,
                offset: None,
                thinking_offset: None,
                limit: None,
            },
            &[],
        )
        .await
    {
        Ok(RecoveredContent::Message { message, .. }) => Some(*message),
        _ => None,
    }
}

/// Read handles of an ephemeral session with `messages` published.
pub(crate) fn ephemeral_read_handles(
    messages: &[crate::domain::message::Message],
) -> crate::interface::cli::uds_session_handles::SessionReadHandles {
    read_handles_for("", None, messages)
}

/// The composed `list_sessions` handle a dispatch rig holds over `base`.
pub(crate) fn list_handle(
    base: &std::path::Path,
) -> std::sync::Arc<crate::interface::uds::sessions::controller::ListSessionsController> {
    composed_sessions(base).list_sessions
}
