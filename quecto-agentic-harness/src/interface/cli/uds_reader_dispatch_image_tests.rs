//! #2422: a busy agent's reader admits a steer's images once; a steer whose
//! images would be refused leaves the running turn alone, and a forwarded
//! one is refused at once with its receipt recorded by dispatch.
use super::*;

/// A 2x2 PNG: signature, IHDR, IEND.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAAElFTkSuQmCC";

struct Reader {
    registry: crate::interface::cli::uds_ext_protocol::ClientToolRegistry,
    replies: tokio::sync::mpsc::Receiver<String>,
    commands: tokio::sync::mpsc::Sender<ClientMessage>,
    received: tokio::sync::mpsc::Receiver<ClientMessage>,
    cancel: crate::interface::cli::uds_cancel::CancelHandle,
    cancel_rx: tokio::sync::oneshot::Receiver<()>,
    control: crate::interface::cli::uds_cancel::TurnControl,
}

/// A reader over a running turn: its cancel slot is armed.
fn busy_reader() -> Reader {
    let registry = crate::interface::cli::uds_ext_protocol::new_client_tool_registry();
    let (writer, replies) = tokio::sync::mpsc::channel(4);
    crate::interface::cli::uds_ext_protocol::register_client_writer(&registry, 1, writer);
    let (commands, received) = tokio::sync::mpsc::channel(4);
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
    let cancel = std::sync::Arc::new(std::sync::Mutex::new(
        crate::interface::cli::uds_cancel::CancelSlot::Armed(cancel_tx),
    ));
    Reader {
        registry,
        replies,
        commands,
        received,
        cancel,
        cancel_rx,
        control: crate::interface::cli::uds_cancel::TurnControl::default(),
    }
}

impl Reader {
    async fn read(&mut self, line: String) {
        let session =
            crate::interface::cli::uds::dispatch_session_roster_tests::ephemeral_read_handles(&[]);
        assert!(
            dispatch(ReaderDispatchCtx {
                line,
                cancel_handle: &self.cancel,
                turn_control: &self.control,
                session: &session,
                registry: &self.registry,
                subagent_registry: &None,
                fleet: None,
                client_id: 1,
                cmd_tx: &self.commands,
            })
            .await
        );
    }

    fn turn_left_alone(&mut self) -> bool {
        !self.control.is_steer_pending()
            && matches!(
                self.cancel_rx.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            )
    }
}

fn steer(mime: &str, ack: bool) -> String {
    let ack = if ack { r#","ack":"accept""# } else { "" };
    format!(
        r#"{{"type":"steer","id":"s-1","message":"look","images":[{{"mimeType":"{mime}","data":"{PNG}"}}]{ack}}}"#
    )
}

#[tokio::test]
async fn a_refused_busy_steer_leaves_the_running_turn_alone() {
    let mut reader = busy_reader();
    reader.read(steer("image/gif", false)).await;
    assert!(reader.turn_left_alone(), "a refused steer interrupted");
    let ClientMessage::Command(command) = reader.received.try_recv().unwrap() else {
        panic!("dispatch refuses it with its exact message")
    };
    assert!(command.admitted.is_none());
}

#[tokio::test]
async fn an_admitted_busy_steer_interrupts_and_carries_its_admitted_images() {
    let mut reader = busy_reader();
    reader.read(steer("image/png", false)).await;
    assert!(reader.control.is_steer_pending());
    assert!(reader.cancel_rx.try_recv().is_ok(), "the turn is cancelled");
    let ClientMessage::Command(command) = reader.received.try_recv().unwrap() else {
        panic!("command expected")
    };
    let admitted = command.admitted.expect("decoded once, by the reader");
    assert_eq!(admitted[0].data, PNG);
}

#[tokio::test]
async fn a_refused_forwarded_steer_is_refused_at_once_and_its_receipt_sent_to_dispatch() {
    let mut reader = busy_reader();
    reader.read(steer("image/webp", true)).await;
    assert!(reader.turn_left_alone(), "a refused forward interrupted");
    let ack: serde_json::Value =
        serde_json::from_str(reader.replies.try_recv().unwrap().trim()).unwrap();
    assert_eq!(ack["success"], false);
    assert_eq!(
        ack["error"],
        "images[0]: data does not start with the image/webp signature"
    );
    let ClientMessage::RejectedControl(refused) = reader.received.try_recv().unwrap() else {
        panic!("the refusal's receipt goes to dispatch")
    };
    assert_eq!(
        (refused.id.as_str(), refused.command.as_str()),
        ("s-1", "steer")
    );
    assert!(reader.received.try_recv().is_err(), "nothing is forwarded");
}

#[tokio::test]
async fn an_admitted_forwarded_steer_carries_its_admitted_images() {
    let mut reader = busy_reader();
    reader.read(steer("image/png", true)).await;
    assert!(reader.control.is_steer_pending());
    let ClientMessage::Command(command) = reader.received.try_recv().unwrap() else {
        panic!("command expected")
    };
    assert_eq!(command.admitted.expect("admitted once")[0].data, PNG);
}
