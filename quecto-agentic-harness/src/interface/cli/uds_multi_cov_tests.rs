use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};

#[test]
fn busy_guard_sets_flag_and_clears_on_drop() {
    let flag: BusyFlag = Arc::new(AtomicBool::new(false));
    {
        let _guard = BusyGuard::new(&flag);
        assert!(flag.load(Ordering::SeqCst));
    }
    assert!(!flag.load(Ordering::SeqCst));
}

#[tokio::test]
async fn client_guard_drop_decrements_and_sends_disconnect() {
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    let live = Arc::new(AtomicU32::new(1));
    let guard = ClientGuard {
        live_clients: live.clone(),
        cmd_tx: tx,
        client_id: 42,
    };
    drop(guard);

    assert_eq!(live.load(Ordering::SeqCst), 0);
    match rx.recv().await.unwrap() {
        ClientMessage::Disconnected(disconnected) => assert_eq!(disconnected.client_id, 42),
        ClientMessage::Command(_) => panic!("expected disconnect sentinel"),
    }
}

#[tokio::test]
async fn client_guard_drop_ignores_closed_channel() {
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    drop(rx);
    let live = Arc::new(AtomicU32::new(1));
    let guard = ClientGuard {
        live_clients: live.clone(),
        cmd_tx: tx,
        client_id: 7,
    };
    drop(guard);
    assert_eq!(live.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn recv_next_message_prefers_client_message_when_ready() {
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(2);
    let (_notif_tx, notif_rx) =
        crate::infrastructure::tools::subagent_registry::new_notification_channel();
    cmd_tx
        .send(ClientMessage::Command(ClientCommand {
            line: r#"{"type":"get_state"}"#.to_string(),
            client_id: 9,
        }))
        .await
        .unwrap();
    let mut maybe_rx = Some(notif_rx);

    match recv_next_message(&mut cmd_rx, &mut maybe_rx).await.unwrap() {
        DispatchMsg::Client(ClientMessage::Command(command)) => {
            assert_eq!(command.client_id, 9);
            assert!(command.line.contains("get_state"));
        }
        _ => panic!("expected client command"),
    }
}

#[tokio::test]
async fn recv_next_message_returns_notification_when_no_client_ready() {
    let (_cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(1);
    let (notif_tx, notif_rx) =
        crate::infrastructure::tools::subagent_registry::new_notification_channel();
    notif_tx
        .send(
            crate::infrastructure::tools::subagent_registry::SequencedSubagentNotification::new(
                3,
                crate::infrastructure::tools::subagent_registry::SubagentNotification::Completed {
                    agent_id: "child".to_string(),
                },
            ),
        )
        .await
        .unwrap();
    let mut maybe_rx = Some(notif_rx);

    match recv_next_message(&mut cmd_rx, &mut maybe_rx).await.unwrap() {
        DispatchMsg::Notification(notification) => {
            assert_eq!(notification.sequence, 3);
            assert_eq!(notification.dedupe_key().0, "child");
        }
        DispatchMsg::Client(_) => panic!("expected notification"),
    }
}

#[tokio::test]
async fn recv_next_message_returns_none_when_command_channel_closed() {
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(1);
    drop(cmd_tx);
    let mut no_notifications = None;
    assert!(
        recv_next_message(&mut cmd_rx, &mut no_notifications)
            .await
            .is_none()
    );
}

#[test]
fn client_message_variants_carry_client_ids() {
    let cmd = ClientMessage::Command(ClientCommand {
        line: "line".to_string(),
        client_id: 11,
    });
    let disc = ClientMessage::Disconnected(ClientDisconnected { client_id: 12 });
    match cmd {
        ClientMessage::Command(command) => assert_eq!(command.client_id, 11),
        ClientMessage::Disconnected(_) => panic!("expected command"),
    }
    match disc {
        ClientMessage::Disconnected(disconnected) => assert_eq!(disconnected.client_id, 12),
        ClientMessage::Command(_) => panic!("expected disconnect"),
    }
}

#[tokio::test]
async fn handle_client_routes_broadcast_targeted_lag_and_reader_commands() {
    let (server, client) = tokio::net::UnixStream::pair().unwrap();
    let (broadcast_tx, broadcast_rx) = tokio::sync::broadcast::channel::<String>(1);
    broadcast_tx.send(r#"{"type":"old"}"#.to_string()).unwrap();
    broadcast_tx.send(r#"{"type":"new"}"#.to_string()).unwrap();
    let (targeted_tx, targeted_rx) = tokio::sync::mpsc::channel::<String>(2);
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<ClientMessage>(4);
    let guard_tx = cmd_tx.clone();
    let live = Arc::new(AtomicU32::new(1));
    let registry = super::super::uds_ext_protocol::new_client_tool_registry();
    let turn_control: super::super::uds_cancel::TurnControlHandle = Arc::default();
    let cancel_handle: super::super::uds_cancel::CancelHandle = Arc::new(std::sync::Mutex::new(
        super::super::uds_cancel::CancelSlot::Idle,
    ));
    let snapshot = Arc::new(tokio::sync::RwLock::new(
        super::super::uds_snapshots::ConversationSnapshotData::default(),
    ));

    let task = tokio::spawn(handle_client(ClientHandlerArgs {
        stream: server,
        broadcast_rx,
        targeted_rx,
        cmd_tx,
        cancel_handle,
        turn_control: turn_control.clone(),
        client_id: 77,
        client_tool_registry: registry,
        conversation_snapshot: snapshot,
        subagent_registry: None,
        _guard: ClientGuard {
            live_clients: live.clone(),
            cmd_tx: guard_tx,
            client_id: 77,
        },
    }));

    let (reader, mut writer) = tokio::io::split(client);
    let mut lines = tokio::io::BufReader::new(reader).lines();

    let lag_line = lines.next_line().await.unwrap().unwrap();
    assert!(lag_line.contains("dropped 1 events"), "{lag_line}");
    let new_line = lines.next_line().await.unwrap().unwrap();
    assert!(new_line.contains(r#""type":"new""#), "{new_line}");

    targeted_tx
        .send(r#"{"type":"execute_tool","toolName":"owned"}"#.to_string())
        .await
        .unwrap();
    let targeted_line = lines.next_line().await.unwrap().unwrap();
    assert!(targeted_line.contains("execute_tool"), "{targeted_line}");

    writer
        .write_all(
            br#"{"type":"abort","id":"a1"}
{"type":"get_state","id":"s1"}
"#,
        )
        .await
        .unwrap();
    writer.shutdown().await.unwrap();

    match cmd_rx.recv().await.unwrap() {
        ClientMessage::Command(command) => {
            assert_eq!(command.client_id, 77);
            assert!(command.line.contains("abort"));
        }
        ClientMessage::Disconnected(_) => panic!("expected abort command first"),
    }
    assert!(turn_control.is_abort_pending());
    match cmd_rx.recv().await.unwrap() {
        ClientMessage::Command(command) => assert!(command.line.contains("get_state")),
        ClientMessage::Disconnected(_) => panic!("expected get_state command second"),
    }
    match cmd_rx.recv().await.unwrap() {
        ClientMessage::Disconnected(disconnected) => assert_eq!(disconnected.client_id, 77),
        ClientMessage::Command(_) => panic!("expected disconnect sentinel"),
    }
    assert_eq!(live.load(Ordering::SeqCst), 0);
    task.await.unwrap();
}

#[tokio::test]
async fn handle_client_closes_on_version_mismatch_and_drops_guard() {
    let (server, mut client) = tokio::net::UnixStream::pair().unwrap();
    let (broadcast_tx, broadcast_rx) = tokio::sync::broadcast::channel::<String>(1);
    let (_targeted_tx, targeted_rx) = tokio::sync::mpsc::channel::<String>(1);
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<ClientMessage>(2);
    let live = Arc::new(AtomicU32::new(1));
    let registry = super::super::uds_ext_protocol::new_client_tool_registry();
    let snapshot = Arc::new(tokio::sync::RwLock::new(
        super::super::uds_snapshots::ConversationSnapshotData::default(),
    ));

    let task = tokio::spawn(handle_client(ClientHandlerArgs {
        stream: server,
        broadcast_rx,
        targeted_rx,
        cmd_tx: cmd_tx.clone(),
        cancel_handle: Arc::new(std::sync::Mutex::new(
            super::super::uds_cancel::CancelSlot::Idle,
        )),
        turn_control: Arc::default(),
        client_id: 88,
        client_tool_registry: registry,
        conversation_snapshot: snapshot,
        subagent_registry: None,
        _guard: ClientGuard {
            live_clients: live.clone(),
            cmd_tx,
            client_id: 88,
        },
    }));
    drop(broadcast_tx);

    client.write_all(&[0xFF, 0, 0, 0]).await.unwrap();
    task.await.unwrap();
    assert_eq!(live.load(Ordering::SeqCst), 0);
    match cmd_rx.recv().await.unwrap() {
        ClientMessage::Disconnected(disconnected) => assert_eq!(disconnected.client_id, 88),
        ClientMessage::Command(_) => panic!("expected disconnect only"),
    }
    assert!(cmd_rx.try_recv().is_err());
}

#[tokio::test]
async fn final_roster_snapshot_preserves_completed_ordinary_exit_barrier() {
    use crate::domain::message::Message;
    use crate::domain::session::{
        PersistedSubagentRosterEntry, Session, SessionStore, SubagentLiveness,
        SubagentRestoreReason,
    };
    use crate::infrastructure::persistence::session_store::FileSessionStore;
    use crate::infrastructure::tools::subagent_registry::new_registry;

    let tmp = tempfile::TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    store
        .save(&Session {
            key: "cli:test".into(),
            messages: vec![Message::user("saved")],
            workflow_run: None,
            subagent_roster: vec![PersistedSubagentRosterEntry {
                agent_uuid: "child".into(),
                display_name: "child".into(),
                session_key: "child".into(),
                socket_path: "/tmp/child.sock".into(),
                pid: 123,
                liveness: SubagentLiveness::Dead,
                restore_reason: SubagentRestoreReason::OrdinaryTuiExitStopped,
                parent_id: None,
                read_only: false,
                delivered_message_ordinal: None,
                pending_message_reports: std::collections::VecDeque::new(),
                status: None,
            }],
        })
        .await
        .unwrap();

    let registry = Some(new_registry());
    let roster = final_subagent_roster_snapshot(&store, "cli:test", &registry).await;

    assert_eq!(roster.len(), 1);
    assert_eq!(
        roster[0].restore_reason,
        SubagentRestoreReason::OrdinaryTuiExitStopped
    );
}
