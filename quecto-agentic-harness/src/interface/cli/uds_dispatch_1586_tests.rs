use super::cov_tests::Fixture;
use super::{
    dispatch_command, persist_current_session, persist_current_session_with_restore_reason,
};
use crate::domain::ids::AgentUuid;
use crate::domain::message::Message;
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};
use crate::interface::cli::protocol::AgentCommand;

fn serve_session_stats_once(
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
            "data": { "sessionKey": session_key }
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
fn legacy_ordinary_exit_marker_restores_only_verified_live_socket_identity() {
    use crate::domain::session::{SubagentLiveness, SubagentRestoreReason};
    use crate::interface::cli::uds::uds_dispatch_session::restore_persisted_subagent_roster;

    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("compat-live.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let server = serve_session_stats_once(listener, "compat-live");

    let compat = crate::domain::session::PersistedSubagentRosterEntry {
        agent_uuid: "compat-live".into(),
        display_name: "Compat live".into(),
        session_key: "compat-live".into(),
        socket_path: socket,
        pid: 99,
        liveness: SubagentLiveness::Live,
        restore_reason: SubagentRestoreReason::OrdinaryTuiExitStopped,
        parent_id: Some("parent".into()),
        read_only: true,
        delivered_message_ordinal: Some(7),
        pending_message_reports: std::collections::VecDeque::new(),
        status: Some("idle".into()),
    };
    let mut exited_control = compat.clone();
    exited_control.agent_uuid = "exited-control".into();
    exited_control.session_key = "exited-control".into();
    exited_control.status = Some("exited".into());

    let registry = new_registry();
    restore_persisted_subagent_roster(&Some(registry.clone()), vec![compat, exited_control]);

    let entries = registry.lock().unwrap();
    assert_eq!(entries.len(), 1);
    let restored = entries
        .get("compat-live")
        .expect("verified live ordinary marker restored");
    assert_eq!(restored.persisted_liveness, SubagentLiveness::Live);
    assert_eq!(restored.status.to_wire_str(), "idle");
    assert_eq!(restored.display_name, "Compat live");
    assert!(!entries.contains_key("exited-control"));
    drop(entries);
    server.join().unwrap();
}

#[test]
fn killing_exit_empty_restore_cycles_stay_empty_but_new_live_registration_appears() {
    use crate::domain::session::SubagentRestoreReason;
    use crate::interface::cli::protocol::build_compact_subagent_roster;
    use crate::interface::cli::uds::uds_dispatch_session::{
        restore_persisted_subagent_roster, snapshot_subagent_roster_with_restore_reason,
    };

    let registry = new_registry();
    for _ in 0..3 {
        let snapshot = snapshot_subagent_roster_with_restore_reason(
            &Some(registry.clone()),
            SubagentRestoreReason::OrdinaryTuiExitStopped,
        );
        assert!(snapshot.is_empty());
        restore_persisted_subagent_roster(&Some(registry.clone()), snapshot);
        assert!(
            build_compact_subagent_roster(&Some(registry.clone()), None)
                .unwrap()
                .subagents
                .is_empty()
        );
    }

    let mut child = SubagentEntry::with_identity(
        AgentUuid::from("new-child".to_string()),
        "New child".to_string(),
        "/tmp/new-child.sock".into(),
        123,
    );
    child.status = crate::infrastructure::tools::subagent_registry::SubagentStatus::Idle;
    registry.lock().unwrap().insert("new-child".into(), child);

    let roster = build_compact_subagent_roster(&Some(registry), None).unwrap();
    assert_eq!(roster.subagents.len(), 1);
    assert_eq!(roster.subagents[0].agent_id, "New child");
}

#[tokio::test]
async fn persist_session_unknown_restore_reason_matches_omitted_legacy_behavior() {
    use crate::domain::session::{SessionStore, SubagentRestoreReason};

    async fn saved_reason(reason: SubagentRestoreReason) -> SubagentRestoreReason {
        let mut fx = Fixture::new();
        let registry = new_registry();
        {
            let mut entries = registry.lock().unwrap();
            entries.insert(
                "child-a".into(),
                SubagentEntry::with_identity(
                    AgentUuid::from("child-a".to_string()),
                    "child-a".to_string(),
                    "/tmp/child-a.sock".into(),
                    123,
                ),
            );
        }
        fx.messages = vec![Message::user("hello")];
        {
            let mut ctx = fx.ctx();
            ctx.subagent_registry = Some(registry);
            persist_current_session_with_restore_reason(&mut ctx, reason)
                .await
                .unwrap();
        }
        fx.store
            .load("cli:test")
            .await
            .unwrap()
            .unwrap()
            .subagent_roster[0]
            .restore_reason
    }

    let omitted = saved_reason(SubagentRestoreReason::LegacyUnspecified).await;
    let unknown = saved_reason(SubagentRestoreReason::Unknown).await;
    assert_eq!(unknown, omitted);
    assert_ne!(unknown, SubagentRestoreReason::OrdinaryTuiExitStopped);
}

#[tokio::test]
async fn persist_session_omitted_restore_reason_uses_legacy_behavior() {
    use crate::domain::session::{SessionStore, SubagentRestoreReason};

    let mut fx = Fixture::new();
    let registry = new_registry();
    {
        let mut entries = registry.lock().unwrap();
        entries.insert(
            "child-a".into(),
            SubagentEntry::with_identity(
                AgentUuid::from("child-a".to_string()),
                "child-a".to_string(),
                "/tmp/child-a.sock".into(),
                123,
            ),
        );
    }
    fx.messages = vec![Message::user("hello")];
    {
        let mut ctx = fx.ctx();
        ctx.subagent_registry = Some(registry);
        persist_current_session_with_restore_reason(
            &mut ctx,
            SubagentRestoreReason::LegacyUnspecified,
        )
        .await
        .unwrap();
    }

    let loaded = fx.store.load("cli:test").await.unwrap().unwrap();
    assert_eq!(loaded.subagent_roster.len(), 1);
    assert_eq!(
        loaded.subagent_roster[0].restore_reason,
        SubagentRestoreReason::LegacyUnspecified
    );
}

#[tokio::test]
async fn persist_session_empty_roster_replaces_stale_same_session_only() {
    use crate::domain::session::{
        PersistedSubagentRosterEntry, Session, SessionStore, SubagentLiveness,
        SubagentRestoreReason,
    };

    let mut fx = Fixture::new();
    fx.store
        .save(&Session {
            key: "cli:test".into(),
            messages: vec![Message::user("old-a")],
            workflow_run: None,
            subagent_roster: vec![PersistedSubagentRosterEntry {
                agent_uuid: "stale-child".into(),
                display_name: "stale child".into(),
                session_key: "stale-child".into(),
                socket_path: "/tmp/stale.sock".into(),
                pid: 0,
                liveness: SubagentLiveness::Dead,
                restore_reason: SubagentRestoreReason::LegacyUnspecified,
                parent_id: None,
                read_only: false,
                delivered_message_ordinal: None,
                pending_message_reports: std::collections::VecDeque::new(),
                status: None,
            }],
        })
        .await
        .unwrap();
    fx.store
        .save(&Session {
            key: "cli:other".into(),
            messages: vec![Message::user("old-b")],
            workflow_run: None,
            subagent_roster: vec![PersistedSubagentRosterEntry {
                agent_uuid: "other-child".into(),
                display_name: "other child".into(),
                session_key: "other-child".into(),
                socket_path: "/tmp/other.sock".into(),
                pid: 0,
                liveness: SubagentLiveness::Live,
                restore_reason: SubagentRestoreReason::LegacyUnspecified,
                parent_id: None,
                read_only: false,
                delivered_message_ordinal: None,
                pending_message_reports: std::collections::VecDeque::new(),
                status: None,
            }],
        })
        .await
        .unwrap();

    fx.messages = vec![Message::user("new-a")];
    {
        let mut ctx = fx.ctx();
        ctx.subagent_registry = Some(new_registry());
        persist_current_session(&mut ctx).await.unwrap();
    }

    let current = fx.store.load("cli:test").await.unwrap().unwrap();
    assert!(current.subagent_roster.is_empty());
    assert_eq!(current.messages[0].content, "new-a");

    let other = fx.store.load("cli:other").await.unwrap().unwrap();
    assert_eq!(other.subagent_roster.len(), 1);
    assert_eq!(other.subagent_roster[0].agent_uuid, "other-child");
}

#[tokio::test]
async fn killing_exit_preserves_transcript_without_operational_roster() {
    use crate::domain::session::{
        PersistedSubagentRosterEntry, Session, SessionStore, SubagentLiveness,
        SubagentRestoreReason,
    };

    let mut fx = Fixture::new();
    fx.store
        .save(&Session {
            key: "cli:test".into(),
            messages: vec![Message::user("old")],
            workflow_run: None,
            subagent_roster: vec![PersistedSubagentRosterEntry {
                agent_uuid: "stale-child".into(),
                display_name: "stale child".into(),
                session_key: "stale-child".into(),
                socket_path: "/tmp/stale.sock".into(),
                pid: 0,
                liveness: SubagentLiveness::Dead,
                restore_reason: SubagentRestoreReason::LegacyUnspecified,
                parent_id: None,
                read_only: false,
                delivered_message_ordinal: None,
                pending_message_reports: std::collections::VecDeque::new(),
                status: None,
            }],
        })
        .await
        .unwrap();

    let registry = new_registry();
    {
        let mut entries = registry.lock().unwrap();
        entries.insert(
            "fresh-child".into(),
            SubagentEntry::with_identity(
                AgentUuid::from("fresh-child".to_string()),
                "fresh child".to_string(),
                "/tmp/fresh.sock".into(),
                456,
            ),
        );
    }
    fx.messages = vec![Message::user("new")];
    {
        let mut ctx = fx.ctx();
        ctx.subagent_registry = Some(registry);
        persist_current_session_with_restore_reason(
            &mut ctx,
            SubagentRestoreReason::OrdinaryTuiExitStopped,
        )
        .await
        .unwrap();
    }

    let current = fx.store.load("cli:test").await.unwrap().unwrap();
    assert!(current.subagent_roster.is_empty());
    assert_eq!(current.messages.len(), 1);
    assert_eq!(current.messages[0].content, "new");
}

#[tokio::test]
async fn persist_session_dispatch_success_emits_correlated_ok_event() {
    use crate::domain::session::SessionStore;

    let mut fx = Fixture::new();
    fx.messages = vec![Message::user("persist me")];
    let (tx, mut rx) = tokio::sync::broadcast::channel(4);
    {
        let mut ctx = fx.ctx();
        ctx.broadcast_tx = Some(tx);
        ctx.stdout = None;
        dispatch_command(
            AgentCommand::PersistSession {
                id: Some("persist-1".into()),
                restore_reason: None,
            },
            &mut ctx,
        )
        .await;
    }

    let event: serde_json::Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
    assert_eq!(event["type"], "response");
    assert_eq!(event["id"], "persist-1");
    assert_eq!(event["command"], "persist_session");
    assert_eq!(event["success"], true);
    assert_eq!(
        fx.store.load("cli:test").await.unwrap().unwrap().messages[0].content,
        "persist me"
    );
}

#[tokio::test]
async fn persist_session_dispatch_failure_emits_correlated_err_event() {
    let mut fx = Fixture::new();
    fx.messages = vec![Message::user("persist me")];
    let occupied = fx._tmp.path().join("sessions");
    tokio::fs::write(&occupied, b"not a directory")
        .await
        .unwrap();
    let (tx, mut rx) = tokio::sync::broadcast::channel(4);
    {
        let mut ctx = fx.ctx();
        ctx.broadcast_tx = Some(tx);
        ctx.stdout = None;
        dispatch_command(
            AgentCommand::PersistSession {
                id: Some("persist-fail".into()),
                restore_reason: None,
            },
            &mut ctx,
        )
        .await;
    }

    let event: serde_json::Value = serde_json::from_str(&rx.recv().await.unwrap()).unwrap();
    assert_eq!(event["type"], "response");
    assert_eq!(event["id"], "persist-fail");
    assert_eq!(event["command"], "persist_session");
    assert_eq!(event["success"], false);
    assert!(event["error"].as_str().unwrap().contains("failed"));
}

#[tokio::test]
async fn killing_barrier_survives_routine_saves_without_clearing_live_registry() {
    use crate::domain::session::{SessionStore, SubagentRestoreReason};
    let mut fx = Fixture::new();
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "live".into(),
        SubagentEntry::new("/tmp/live.sock".into(), 123),
    );
    fx.messages = vec![Message::user("keep transcript")];
    {
        let mut ctx = fx.ctx();
        ctx.subagent_registry = Some(registry.clone());
        persist_current_session_with_restore_reason(
            &mut ctx,
            SubagentRestoreReason::OrdinaryTuiExitStopped,
        )
        .await
        .unwrap();
        persist_current_session(&mut ctx).await.unwrap();
    }
    assert!(
        fx.store
            .load("cli:test")
            .await
            .unwrap()
            .unwrap()
            .subagent_roster
            .is_empty(),
        "routine save must not undo a killing barrier"
    );
    assert_eq!(
        registry.lock().unwrap().len(),
        1,
        "persistence does not mutate live reconnect state"
    );
}

#[tokio::test]
async fn explicit_detach_clears_killing_intent_and_session_switch_resets_it() {
    use crate::domain::session::{SessionStore, SubagentRestoreReason};
    let mut fx = Fixture::new();
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "live".into(),
        SubagentEntry::new("/tmp/live.sock".into(), 123),
    );
    {
        let mut ctx = fx.ctx();
        ctx.subagent_registry = Some(registry);
        persist_current_session_with_restore_reason(
            &mut ctx,
            SubagentRestoreReason::OrdinaryTuiExitStopped,
        )
        .await
        .unwrap();
        persist_current_session_with_restore_reason(
            &mut ctx,
            SubagentRestoreReason::LegacyUnspecified,
        )
        .await
        .unwrap();
        assert!(!ctx.session.killing_exit);
        ctx.session.killing_exit = true;
        ctx.session.set_session_key("cli:another".into());
        assert!(!ctx.session.killing_exit);
    }
    assert_eq!(
        fx.store
            .load("cli:test")
            .await
            .unwrap()
            .unwrap()
            .subagent_roster
            .len(),
        1
    );
}
