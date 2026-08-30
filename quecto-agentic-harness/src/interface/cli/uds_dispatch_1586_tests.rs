use super::cov_tests::Fixture;
use super::{
    dispatch_command, persist_current_session, persist_current_session_with_restore_reason,
};
use crate::domain::ids::AgentUuid;
use crate::domain::message::Message;
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};
use crate::interface::cli::protocol::AgentCommand;

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
async fn persist_session_non_empty_roster_replaces_stale_same_session_only() {
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
    let ids: Vec<_> = current
        .subagent_roster
        .iter()
        .map(|entry| entry.agent_uuid.as_str())
        .collect();
    assert_eq!(ids, vec!["fresh-child"]);
    assert_eq!(
        current.subagent_roster[0].restore_reason,
        SubagentRestoreReason::OrdinaryTuiExitStopped
    );
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
