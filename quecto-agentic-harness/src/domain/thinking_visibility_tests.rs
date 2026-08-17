use crate::domain::message::{Message, ThinkingBlock};
use crate::interface::cli::uds_session::message_to_json;

#[test]
fn message_view_exposes_display_safe_thinking_without_private_replay_fields() {
    let mut message = Message::assistant("final answer", vec![]);
    message.thinking_blocks = vec![
        ThinkingBlock::Normal {
            thinking: "visible reasoning".into(),
            signature: "PRIVATE_SIGNATURE".into(),
        },
        ThinkingBlock::Redacted {
            data: "PRIVATE_REDACTED_BLOB".into(),
        },
    ];

    let view = message_to_json(&message);
    assert_eq!(view["visibleThinking"][0]["text"], "visible reasoning");
    assert_eq!(view["visibleThinking"][1]["text"], "[Redacted thinking]");

    let wire = view.to_string();
    assert!(!wire.contains("PRIVATE_SIGNATURE"));
    assert!(!wire.contains("PRIVATE_REDACTED_BLOB"));
}
