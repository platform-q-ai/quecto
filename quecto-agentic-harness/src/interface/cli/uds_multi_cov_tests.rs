use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

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
