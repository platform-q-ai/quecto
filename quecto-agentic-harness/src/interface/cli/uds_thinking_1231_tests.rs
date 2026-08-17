use super::uds_session::{MessageView, message_to_json};
use crate::domain::message::{Message, ThinkingBlock};

#[test]
fn message_view_exposes_display_safe_thinking_separate_from_answer() {
    let mut msg = Message::assistant("final answer", vec![]);
    msg.thinking_blocks.push(ThinkingBlock::Normal {
        thinking: "visible reasoning".into(),
        signature: "private-signature".into(),
    });
    msg.thinking_blocks.push(ThinkingBlock::Redacted {
        data: "opaque-redacted-payload".into(),
    });

    let v = serde_json::to_value(MessageView(&msg)).expect("MessageView serializes");
    assert_eq!(v["content"], "final answer");
    assert_eq!(v["thinking"][0]["kind"], "text");
    assert_eq!(v["thinking"][0]["text"], "visible reasoning");
    assert_eq!(v["thinking"][1]["kind"], "redacted");
    let wire = serde_json::to_string(&v).unwrap();
    assert!(!wire.contains("private-signature"));
    assert!(!wire.contains("opaque-redacted-payload"));
}

#[test]
fn message_to_json_omits_thinking_when_absent_for_additive_recovery() {
    let msg = Message::assistant("answer only", vec![]);
    let v = message_to_json(&msg);
    assert_eq!(v["content"], "answer only");
    assert!(v.get("thinking").is_none());
}
