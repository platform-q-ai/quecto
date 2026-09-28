//! What a member writes to `claude`'s stream-json input (#2286, #2287):
//! one line per user turn, and one per interrupt, in the shapes the CLI's
//! own SDK sends (`SDKUserMessage`, and the `interrupt` control request).

use serde_json::json;

use crate::application::external_agent::dto::UserTurnId;

/// One user turn as the stream-json user message, one line, under a fresh
/// id: the CLI names the turns a `result` consumed by these ids
/// (`user_message_uuids`).
pub(super) fn user_message_line(text: &str) -> (UserTurnId, String) {
    let id = uuid::Uuid::new_v4().to_string();
    let message = json!({
        "type": "user",
        "uuid": id,
        "message": {"role": "user", "content": [{"type": "text", "text": text}]},
    });
    (UserTurnId(id), one_line(&message))
}

/// An interrupt: it stops the running turn and, with `cancel_queued`,
/// withdraws every user turn still queued (the CLI's
/// `interrupt_cancel_queued_v1`), which its `control_response` names.
pub(super) fn interrupt_line() -> String {
    one_line(&json!({
        "type": "control_request",
        "request_id": uuid::Uuid::new_v4().to_string(),
        "request": {"subtype": "interrupt", "cancel_queued": true},
    }))
}

fn one_line(value: &serde_json::Value) -> String {
    let mut line = value.to_string();
    assert!(!line.contains('\n'), "an input line is exactly one line");
    line.push('\n');
    line
}
