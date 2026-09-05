use crate::domain::ids::AgentUuid;
use crate::domain::session::{PersistedSubagentRosterEntry, SubagentLiveness};
use crate::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentStatus, new_registry,
};
use crate::interface::cli::uds::uds_dispatch_session::{
    restore_persisted_subagent_roster, snapshot_subagent_roster,
    snapshot_subagent_roster_with_restore_reason, verify_persisted_live_subagent,
};

fn roster_entry(id: &str, socket_path: std::path::PathBuf) -> PersistedSubagentRosterEntry {
    PersistedSubagentRosterEntry {
        agent_uuid: id.to_string(),
        display_name: format!("worker-{id}"),
        session_key: id.to_string(),
        socket_path,
        pid: 1,
        liveness: SubagentLiveness::Live,
        restore_reason: crate::domain::session::SubagentRestoreReason::LegacyUnspecified,
        parent_id: Some("parent".to_string()),
        read_only: true,
        delivered_message_ordinal: None,
        pending_message_reports: std::collections::VecDeque::new(),
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
fn ordinary_exit_snapshot_marks_dead_tombstones_non_restorable() {
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
        let mut killed = SubagentEntry::with_identity(
            AgentUuid::from("killed"),
            "Killed worker".into(),
            "/tmp/killed.sock".into(),
            2,
        );
        killed.status = SubagentStatus::Exited;
        killed.persisted_liveness = SubagentLiveness::Dead;
        entries.insert("killed".into(), killed);
    }

    let roster = snapshot_subagent_roster_with_restore_reason(
        &Some(registry),
        crate::domain::session::SubagentRestoreReason::OrdinaryTuiExitStopped,
    );

    assert!(
        roster.is_empty(),
        "killing exit persists no operational children"
    );

    let restored = new_registry();
    restore_persisted_subagent_roster(&Some(restored.clone()), roster);
    assert!(!restored.lock().unwrap().contains_key("killed"));
}

#[test]
fn restore_rejects_dead_rows_even_if_marked_ordinary_exit_stopped() {
    let mut bad = roster_entry("bad", "/tmp/bad.sock".into());
    bad.restore_reason = crate::domain::session::SubagentRestoreReason::OrdinaryTuiExitStopped;
    bad.liveness = SubagentLiveness::Dead;
    bad.status = Some("exited".into());

    let registry = new_registry();
    restore_persisted_subagent_roster(&Some(registry.clone()), vec![bad]);

    assert!(
        registry.lock().unwrap().is_empty(),
        "defensive restore must not re-show killed/dead rows from stale persisted data"
    );
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

#[tokio::test]
async fn verify_persisted_live_subagent_requires_matching_session_stats_identity() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("child.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (reader, mut writer) = tokio::io::split(stream);
        let mut reader = tokio::io::BufReader::new(reader);
        let request =
            quecto_line_io::read_frame(&mut reader, quecto_line_io::PROTOCOL_FRAME_CAP_BYTES)
                .await
                .unwrap()
                .unwrap();
        let request_id = serde_json::from_slice::<serde_json::Value>(&request).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        let response = serde_json::json!({
            "type": "response",
            "id": request_id,
            "command": "get_session_stats",
            "success": true,
            "data": { "sessionKey": "cli:other", "userMessages": 0, "assistantMessages": 0, "toolCalls": 0, "toolResults": 0, "totalMessages": 0, "tokens": {}, "contextTokens": 0, "maxContextTokens": 0 }
        });
        quecto_line_io::write_frame(
            &mut writer,
            response.to_string().as_bytes(),
            quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
        )
        .await
        .unwrap();
    });

    let mut entry = roster_entry("child", socket);
    entry.session_key = "cli:child".into();
    assert!(
        !tokio::task::spawn_blocking(move || verify_persisted_live_subagent(&entry))
            .await
            .unwrap()
    );
    server.await.unwrap();
}

/// #1474: resume must not rehydrate dead / unverifiable roster rows as grey
/// "ghost" panel agents. Only currently verifiable live agents re-enter the
/// registry; dead and failed-verify live/detached entries are pruned.
fn serve_matching_session_stats(
    listener: std::os::unix::net::UnixListener,
    session_key: &'static str,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        let (mut stream, _) = listener.accept().unwrap();
        let mut prefix = [0u8; 4];
        stream.read_exact(&mut prefix).unwrap();
        let len = u32::from_be_bytes(prefix) as usize;
        let mut request = vec![0u8; len];
        stream.read_exact(&mut request).unwrap();
        let request_id = serde_json::from_slice::<serde_json::Value>(&request).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        let response = serde_json::json!({
            "type": "response",
            "id": request_id,
            "command": "get_session_stats",
            "success": true,
            "data": {
                "sessionKey": session_key,
                "userMessages": 0,
                "assistantMessages": 0,
                "toolCalls": 0,
                "toolResults": 0,
                "totalMessages": 0,
                "tokens": {},
                "contextTokens": 0,
                "maxContextTokens": 0
            }
        })
        .to_string();
        stream
            .write_all(&(response.len() as u32).to_be_bytes())
            .unwrap();
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    })
}

#[test]
fn restore_persisted_roster_keeps_only_verifiably_live_agents() {
    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let live_socket = dir.path().join("live.sock");
    let detached_live_socket = dir.path().join("detached-live.sock");
    let live_listener = std::os::unix::net::UnixListener::bind(&live_socket).unwrap();
    let detached_listener = std::os::unix::net::UnixListener::bind(&detached_live_socket).unwrap();
    let live_server = serve_matching_session_stats(live_listener, "live");
    let detached_server = serve_matching_session_stats(detached_listener, "still-up");

    let mut live = roster_entry("live", live_socket);
    live.display_name = "Live worker".into();
    live.read_only = false;
    let mut unreachable = roster_entry("gone", dir.path().join("gone.sock"));
    unreachable.display_name = "Gone worker".into();
    let mut dead = roster_entry("dead", dir.path().join("dead.sock"));
    dead.liveness = SubagentLiveness::Dead;
    let mut detached_unreachable = roster_entry("old-container", dir.path().join("old.sock"));
    detached_unreachable.liveness = SubagentLiveness::Detached;
    detached_unreachable.display_name = "Old container ghost".into();
    let mut detached_live = roster_entry("still-up", detached_live_socket);
    detached_live.liveness = SubagentLiveness::Detached;
    detached_live.display_name = "Detached but reachable".into();

    restore_persisted_subagent_roster(
        &Some(registry.clone()),
        vec![live, unreachable, dead, detached_unreachable, detached_live],
    );

    let entries = registry.lock().unwrap();
    assert_eq!(
        entries.len(),
        2,
        "dead and unverifiable live/detached entries must not reappear as ghosts: {:?}",
        entries.keys().collect::<Vec<_>>()
    );
    assert!(
        !entries.contains_key("gone"),
        "unreachable live entry must be pruned on restore"
    );
    assert!(
        !entries.contains_key("dead"),
        "dead entry must be pruned on restore"
    );
    assert!(
        !entries.contains_key("old-container"),
        "detached unreachable entry must be pruned on restore"
    );

    let live_entry = entries.get("live").expect("verified live agent restored");
    assert_eq!(live_entry.persisted_liveness, SubagentLiveness::Live);
    assert_eq!(live_entry.status.to_wire_str(), "idle");
    assert_eq!(live_entry.display_name, "Live worker");

    let detached_entry = entries
        .get("still-up")
        .expect("verified detached-but-reachable agent restored as live");
    assert_eq!(detached_entry.persisted_liveness, SubagentLiveness::Live);
    assert_eq!(detached_entry.status.to_wire_str(), "idle");
    assert_eq!(detached_entry.display_name, "Detached but reachable");

    live_server.join().unwrap();
    detached_server.join().unwrap();
}

/// #1474 N1: verify probes must not hold the shared registry mutex. Resume can
/// stall concurrent get_subagents/spawn registration if lock spans socket IO.
#[test]
fn restore_persisted_roster_does_not_hold_registry_lock_during_verify() {
    use std::io::{Read, Write};
    use std::time::Duration;

    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("slow.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();

    let (connected_tx, connected_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        // Restore has entered verify and is blocked on the response.
        connected_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        let mut prefix = [0u8; 4];
        stream.read_exact(&mut prefix).unwrap();
        let len = u32::from_be_bytes(prefix) as usize;
        let mut request = vec![0u8; len];
        stream.read_exact(&mut request).unwrap();
        let request_id = serde_json::from_slice::<serde_json::Value>(&request).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        let response = serde_json::json!({
            "type": "response",
            "id": request_id,
            "command": "get_session_stats",
            "success": true,
            "data": {
                "sessionKey": "slow",
                "userMessages": 0,
                "assistantMessages": 0,
                "toolCalls": 0,
                "toolResults": 0,
                "totalMessages": 0,
                "tokens": {},
                "contextTokens": 0,
                "maxContextTokens": 0
            }
        })
        .to_string();
        stream
            .write_all(&(response.len() as u32).to_be_bytes())
            .unwrap();
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    });

    let restore_registry = registry.clone();
    let restore = std::thread::spawn(move || {
        restore_persisted_subagent_roster(
            &Some(restore_registry),
            vec![roster_entry("slow", socket)],
        );
    });

    connected_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("verify should connect to the slow socket");
    // While verify is blocked mid-probe, concurrent registry users must not stall.
    let lock_acquired_during_verify = registry.try_lock().is_ok();
    release_tx.send(()).unwrap();
    restore.join().unwrap();
    server.join().unwrap();

    assert!(
        lock_acquired_during_verify,
        "restore must not hold the registry mutex across verify socket IO"
    );
    assert!(
        registry.lock().unwrap().contains_key("slow"),
        "verified agent must still be restored after lock is released during probe"
    );
}

#[test]
fn restore_persisted_roster_no_registry_is_noop() {
    restore_persisted_subagent_roster(&None, vec![roster_entry("ignored", "".into())]);
}

#[test]
fn restore_classifier_respects_explicit_restore_reasons() {
    use crate::domain::session::SubagentRestoreReason;

    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let mut restored = roster_entry("restored", dir.path().join("stale-restored.sock"));
    restored.restore_reason = SubagentRestoreReason::OrdinaryTuiExitStopped;
    restored.pid = 42;
    restored.display_name = "Restored worker".into();
    let mut killed = roster_entry("killed", dir.path().join("killed.sock"));
    killed.restore_reason = SubagentRestoreReason::ExplicitlyKilled;
    let mut legacy_dead = roster_entry("legacy-dead", dir.path().join("legacy-dead.sock"));
    legacy_dead.liveness = SubagentLiveness::Dead;

    restore_persisted_subagent_roster(&Some(registry.clone()), vec![restored, killed, legacy_dead]);

    let entries = registry.lock().unwrap();
    assert!(
        entries.is_empty(),
        "stopped, killed and dead rows do not return"
    );
}

#[test]
fn restored_ordinary_exit_rows_are_not_socket_or_kill_targetable() {
    use crate::domain::session::SubagentRestoreReason;
    use crate::domain::tool::Tool;
    use crate::infrastructure::tools::agent_cmd::AgentCmdTool;

    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let stale_socket = dir.path().join("stale.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&stale_socket).unwrap();
    let mut restored = roster_entry("restored", stale_socket);
    restored.display_name = "Restored worker".into();
    restored.restore_reason = SubagentRestoreReason::OrdinaryTuiExitStopped;
    restore_persisted_subagent_roster(&Some(registry.clone()), vec![restored]);

    let lookup = crate::infrastructure::tools::subagent_registry::lookup_subagent_socket(
        &registry, "restored",
    );
    assert!(lookup.is_err(), "restored row must not expose stale socket");

    let rt = tokio::runtime::Runtime::new().unwrap();
    for agent_ref in ["Restored worker", "restored"] {
        let result = rt
            .block_on(
                AgentCmdTool::new(registry.clone())
                    .execute(&format!(r#"{{"agent_id":"{agent_ref}","command":"kill"}}"#)),
            )
            .unwrap();
        assert!(
            result.is_error,
            "restored row must not be kill-targetable by {agent_ref}"
        );
        assert!(registry.lock().unwrap().is_empty());
    }
}

#[test]
fn restore_is_scoped_to_the_resumed_session_without_bleed() {
    use crate::domain::session::SubagentRestoreReason;

    let dir = tempfile::tempdir().unwrap();
    let registry_a = new_registry();
    let registry_b = new_registry();
    let mut a = roster_entry("agent-a", dir.path().join("a.sock"));
    a.display_name = "same-label".into();
    a.session_key = "session-a".into();
    a.restore_reason = SubagentRestoreReason::OrdinaryTuiExitStopped;
    let mut b = roster_entry("agent-b", dir.path().join("b.sock"));
    b.display_name = "same-label".into();
    b.session_key = "session-b".into();
    b.restore_reason = SubagentRestoreReason::ExplicitlyKilled;

    restore_persisted_subagent_roster(&Some(registry_a.clone()), vec![a.clone()]);
    restore_persisted_subagent_roster(&Some(registry_b.clone()), vec![b.clone()]);

    assert!(registry_a.lock().unwrap().is_empty());
    assert!(!registry_a.lock().unwrap().contains_key("agent-b"));
    assert!(
        registry_b.lock().unwrap().is_empty(),
        "killed row in another session does not bleed or return"
    );
}

#[test]
fn persisted_roster_entry_tolerates_unknown_reason_and_missing_required_legacy_fields() {
    let unknown: PersistedSubagentRosterEntry = serde_json::from_value(serde_json::json!({
        "agentUuid": "future",
        "displayName": "Future worker",
        "sessionKey": "future",
        "socketPath": "/tmp/future.sock",
        "pid": 9,
        "liveness": "live",
        "restoreReason": "future_reason"
    }))
    .unwrap();
    assert_eq!(
        unknown.restore_reason,
        crate::domain::session::SubagentRestoreReason::Unknown
    );

    let malformed: PersistedSubagentRosterEntry = serde_json::from_value(serde_json::json!({
        "displayName": "missing identity"
    }))
    .unwrap();
    assert!(malformed.agent_uuid.is_empty());
    assert_eq!(malformed.liveness, SubagentLiveness::Dead);
    assert_eq!(
        malformed.restore_reason,
        crate::domain::session::SubagentRestoreReason::LegacyUnspecified
    );

    let registry = new_registry();
    restore_persisted_subagent_roster(&Some(registry.clone()), vec![unknown, malformed]);
    assert!(registry.lock().unwrap().is_empty());
}

#[test]
fn ordinary_exit_resume_full_refresh_never_reintroduces_stopped_or_preexisting_dead_children() {
    use crate::domain::session::SubagentRestoreReason;
    use crate::interface::cli::protocol::build_compact_subagent_roster;

    // The exit barrier snapshots before stopping live children. A child already
    // dead at that barrier is deliberately non-restorable (#1608 control case).
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
        restore_persisted_subagent_roster(&Some(restored.clone()), snapshot);
        for _ in 0..3 {
            let roster = build_compact_subagent_roster(&Some(restored.clone()), None).unwrap();
            assert!(
                roster.subagents.is_empty(),
                "killing exit must not restore operational rows"
            );
            assert!(
                crate::interface::cli::protocol::build_live_subagent_info_list(&Some(
                    restored.clone()
                ))
                .is_empty()
            );
            let next = snapshot_subagent_roster_with_restore_reason(
                &Some(restored.clone()),
                SubagentRestoreReason::OrdinaryTuiExitStopped,
            );
            restore_persisted_subagent_roster(&Some(restored.clone()), next);
        }
        assert!(restored.lock().unwrap().is_empty());
    }
}
