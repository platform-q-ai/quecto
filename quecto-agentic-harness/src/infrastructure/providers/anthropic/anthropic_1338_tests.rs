// Issue #1338: system-message handling on the wire. A `Role::System` message
// is not a conversational turn here — it folds into the top-level `system`
// field — so notes injected as system messages never reached the model and
// silently replaced the real system prompt. Split from anthropic_tests.rs for
// the 750-line limit.

use super::*;

/// Anthropic has no in-conversation system role, so every `Role::System`
/// message folds into the one top-level `system` field. Assigning meant the
/// LAST one silently replaced the real system prompt — the pinned spill
/// manifest left the agent running with its whole prompt reduced to one line
/// of recall guidance.
#[test]
fn test_build_messages_concatenates_system_messages_without_clobbering() {
    let messages = vec![
        Message::system("REAL SYSTEM PROMPT"),
        Message::system("[Session memory is available via recall()]"),
        Message::user("hello"),
    ];
    let (sys, api_messages) =
        AnthropicProvider::build_messages(&messages, "claude-opus-4-5", false);
    let sys = sys.expect("system prompt must survive a later system message");
    assert!(
        sys.contains("REAL SYSTEM PROMPT"),
        "the real system prompt must not be clobbered, got: {sys}"
    );
    assert!(
        sys.contains("recall()"),
        "the later system message must still be delivered, got: {sys}"
    );
    assert!(
        sys.find("REAL SYSTEM PROMPT") < sys.find("recall()"),
        "system messages must keep conversation order, got: {sys}"
    );
    assert_eq!(
        api_messages.len(),
        1,
        "system messages stay out of the messages array"
    );
}

/// A sub-agent completion note must reach the wire as a trailing USER turn.
/// As a system message it was hoisted into the `system` field, leaving the
/// request ending on the parent's own assistant turn — the model continued
/// that answer ("OK, will do") instead of acting on the note.
#[test]
fn test_subagent_note_reaches_wire_as_trailing_user_turn() {
    use crate::interface::cli::uds_session::PendingMessage;

    let note = PendingMessage::subagent_notification(
        "researcher".into(),
        1,
        "Sub-agent 'researcher' ended a turn (status: idle).".into(),
        true,
    )
    .into_message();
    let messages = vec![
        Message::system("REAL SYSTEM PROMPT"),
        Message::user("go"),
        Message::assistant("on it", vec![]),
        note,
    ];
    let (sys, api_messages) =
        AnthropicProvider::build_messages(&messages, "claude-opus-4-5", false);
    assert_eq!(
        sys.as_deref(),
        Some("REAL SYSTEM PROMPT"),
        "the note must not rewrite the cached system block"
    );
    let last = api_messages.last().expect("messages must not be empty");
    assert_eq!(
        last["role"], "user",
        "the request must end on a user turn so the model answers the note"
    );
    assert!(
        serde_json::to_string(last)
            .unwrap()
            .contains("subagent_notification"),
        "the note itself must be the trailing turn, got: {last}"
    );
}
