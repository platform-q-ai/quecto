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
    let cancel = std::sync::Arc::new(std::sync::Mutex::new(
        super::super::uds_cancel::CancelSlot::Idle,
    ));
    let control = super::super::uds_cancel::TurnControl::default();
    let delivery = dispatch(ReaderDispatchCtx {
        line: r#"{"type":"prompt","streamingBehavior":"steer","message":"Approved: use schema v2","ack":"accept","id":"approval-1"}"#.into(),
        cancel_handle: &cancel, turn_control: &control,
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

#[tokio::test]
async fn malformed_steer_admission_does_not_cancel_or_gate_later_work() {
    for line in [
        r#"{"type":"steer"}"#,
        r#"{"type":"prompt","message":3,"streamingBehavior":"steer","ack":"accept"}"#,
        r#"{"type":"steer","message":"bad id","id":3,"ack":"accept"}"#,
        r#"{"type":"steer","message":"old","message":"new","id":"s1","ack":"accept"}"#,
    ] {
        let registry = super::super::uds_ext_protocol::new_client_tool_registry();
        let (commands, mut received) = tokio::sync::mpsc::channel(1);
        let snapshot = std::sync::Arc::new(tokio::sync::RwLock::new(Default::default()));
        let (broadcast, _) = tokio::sync::broadcast::channel(4);
        let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel();
        let cancel = std::sync::Arc::new(std::sync::Mutex::new(
            super::super::uds_cancel::CancelSlot::Armed(cancel_tx),
        ));
        let control = super::super::uds_cancel::TurnControl::default();
        assert!(
            dispatch(ReaderDispatchCtx {
                line: line.into(),
                cancel_handle: &cancel,
                turn_control: &control,
                snapshot: &snapshot,
                registry: &registry,
                subagent_registry: &None,
                broadcast_tx: &broadcast,
                client_id: 1,
                cmd_tx: &commands,
            })
            .await
        );
        assert!(
            !control.is_steer_pending(),
            "malformed input gated valid future work: {line}"
        );
        assert!(matches!(
            cancel_rx.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        let ClientMessage::Command(command) = received.recv().await.unwrap() else {
            panic!("command expected")
        };
        assert!(matches!(
            super::super::uds::parse_line(&command.line),
            super::super::uds::LineResult::ParseError(_)
        ));
    }
}
