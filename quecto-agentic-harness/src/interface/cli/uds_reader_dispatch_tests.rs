use super::*;

#[tokio::test]
async fn clarification_is_not_acknowledged_when_dispatch_queue_is_full() {
    rejected_queue(false).await;
}

#[tokio::test]
async fn clarification_is_not_acknowledged_when_dispatch_queue_is_closed() {
    rejected_queue(true).await;
}

async fn rejected_queue(closed: bool) {
    let registry = super::super::uds_ext_protocol::new_client_tool_registry();
    let (writer, mut replies) = tokio::sync::mpsc::channel(4);
    super::super::uds_ext_protocol::register_client_writer(&registry, 1, writer);
    let (commands, _receiver) = tokio::sync::mpsc::channel(1);
    commands
        .send(ClientMessage::Command(ClientCommand {
            line: "occupied".into(),
            client_id: 1,
        }))
        .await
        .unwrap();
    if closed {
        drop(_receiver);
    }
    let snapshot = std::sync::Arc::new(tokio::sync::RwLock::new(Default::default()));
    let (broadcast, _) = tokio::sync::broadcast::channel(4);
    let delivery = dispatch(ReaderDispatchCtx {
        line: r#"{"type":"prompt","streamingBehavior":"steer","message":"Approved: use schema v2","ack":"accept","id":"approval-1"}"#.into(),
        snapshot: &snapshot, registry: &registry, subagent_registry: &None,
        broadcast_tx: &broadcast, client_id: 1, cmd_tx: &commands,
    });
    let receive = async {
        let line = replies.recv().await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["id"], "approval-1");
        assert_eq!(
            value["success"], false,
            "queue rejection must not be reported as accepted"
        );
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        tokio::join!(delivery, receive);
    })
    .await
    .unwrap();
}
