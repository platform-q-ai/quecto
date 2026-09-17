//! `CommandSender` and `Client::send` emit identical bytes (#759).

use super::super::*;
use tokio::sync::mpsc;

#[tokio::test]
async fn command_sender_and_client_send_emit_identical_bytes() {
    // Both senders write through serialize_command, so the same command must
    // appear on the wire byte-for-byte regardless of which path sent it (#759).
    let (tx, mut rx) = mpsc::channel::<String>(4);
    let mut sender = CommandSender { tx };
    let cmd = Command::SetModel {
        id: Some("m-1".into()),
        model: Some("claude".into()),
        provider: None,
        model_id: None,
        persist: None,
    };
    sender.send(&cmd).await.expect("send");
    let from_sender = rx.recv().await.expect("frame");

    let expected = serialize_command(&cmd).expect("serialize");
    assert_eq!(from_sender, expected);
}
