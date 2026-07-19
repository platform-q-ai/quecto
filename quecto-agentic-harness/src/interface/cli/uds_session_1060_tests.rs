//! #1060 — wire-visible stable message identifiers on history/snapshot views.
//!
//! Domain messages already carry UUID ids (#1072). Clients must receive
//! round-trippable keys on `MessageView` / `message_to_json` so end-of-turn
//! refs, history, busy-connect snapshots, and on-demand lookup agree (AC6).

use super::{MessageView, message_to_json};
use crate::domain::message::{Message, ToolCall};

fn wire_id(v: &serde_json::Value) -> Option<&str> {
    v.get("id")
        .or_else(|| v.get("messageId"))
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
}

#[test]
fn message_view_exposes_non_empty_stable_message_id() {
    let msg = Message::assistant("hello from history", vec![]);
    let expected = msg.id().to_string();
    let v = serde_json::to_value(MessageView(&msg)).expect("MessageView serializes");
    let id = wire_id(&v)
        .expect("MessageView must expose a non-empty stable message id on the wire (#1060 AC6)");
    assert_eq!(
        id, expected,
        "wire id must round-trip the domain message UUID"
    );
}

#[test]
fn message_to_json_exposes_stable_id_for_all_roles() {
    let user = Message::user("u");
    let assistant = Message::assistant(
        "a",
        vec![ToolCall {
            id: "c1".into(),
            name: "bash".into(),
            arguments: "{}".into(),
        }],
    );
    let tool = Message::tool("c1", "tool-out");

    for msg in [&user, &assistant, &tool] {
        let v = message_to_json(msg);
        let id = wire_id(&v).expect("message_to_json must expose a stable id");
        assert_eq!(id, msg.id().to_string());
    }
}

#[test]
fn history_message_ids_are_distinct_and_non_empty() {
    let msgs = vec![
        Message::user("q"),
        Message::assistant("a", vec![]),
        Message::tool("x", "r"),
    ];
    let mut seen = std::collections::HashSet::new();
    for m in &msgs {
        let v = message_to_json(m);
        let id = wire_id(&v)
            .expect("each history message must carry a non-empty wire id")
            .to_string();
        assert!(
            seen.insert(id.clone()),
            "message ids must be unique across a conversation: {id}"
        );
    }
    assert_eq!(seen.len(), 3);
}

#[test]
fn wire_id_filters_empty_wire_identifiers() {
    let v = serde_json::json!({ "id": "" });
    assert_eq!(wire_id(&v), None);
    let v = serde_json::json!({ "messageId": "" });
    assert_eq!(wire_id(&v), None);
}
