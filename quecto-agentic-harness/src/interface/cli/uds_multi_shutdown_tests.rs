//! Termination-signal exit for the multi-client dispatch loop: a completed
//! teardown must be observed by `recv_next_message` ahead of client traffic.
use super::*;

#[tokio::test]
async fn completed_shutdown_is_observed_before_pending_client_messages() {
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(4);
    cmd_tx
        .send(ClientMessage::Command(super::ClientCommand {
            client_id: 1,
            line: r#"{"type":"get_state"}"#.to_string(),
        }))
        .await
        .unwrap();
    let (shutdown, notify) = super::super::uds_shutdown::ShutdownRequest::for_tests();
    notify.notify_one();
    let mut no_notifications = None;
    assert!(matches!(
        recv_next_message(&mut cmd_rx, &mut no_notifications, &shutdown).await,
        Some(DispatchMsg::Shutdown)
    ));
}

#[tokio::test]
async fn shutdown_requested_before_the_loop_waits_is_not_lost() {
    let (shutdown, notify) = super::super::uds_shutdown::ShutdownRequest::for_tests();
    notify.notify_one();
    let (_cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<ClientMessage>(1);
    let (_notif_tx, notif_rx) = tokio::sync::mpsc::channel(1);
    let mut with_notifications = Some(notif_rx);
    let msg = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        recv_next_message(&mut cmd_rx, &mut with_notifications, &shutdown),
    )
    .await
    .expect("a retained shutdown request must wake the loop");
    assert!(matches!(msg, Some(DispatchMsg::Shutdown)));
}
