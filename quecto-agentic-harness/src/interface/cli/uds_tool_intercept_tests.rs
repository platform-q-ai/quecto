use crate::domain::agent::AgentProgressEvent;
use crate::domain::message::Message;
use crate::interface::cli::uds_cancel::{EventSink, forward_progress_event_sink};

#[tokio::test]
async fn turn_completed_broadcasts_subagent_messages_appended() {
    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(8);
    forward_progress_event_sink(
        AgentProgressEvent::TurnCompleted {
            messages: vec![
                Message::assistant("hello from turn", vec![]),
                Message::tool("call-1", "tool body"),
            ]
            .into(),
        },
        &mut EventSink::Broadcast(tx),
    )
    .await;
    let line = rx.try_recv().expect("an event should be broadcast");
    assert!(line.contains("subagent_messages_appended"), "got: {line}");
    assert!(line.contains("\"agent_id\":\"\""), "got: {line}");
    // #1060: refs only — no full content re-carry.
    assert!(line.contains("messageRefs"), "got: {line}");
    assert!(!line.contains("hello from turn"), "got: {line}");
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["messageRefs"].as_array().map(|a| a.len()), Some(2));
}
