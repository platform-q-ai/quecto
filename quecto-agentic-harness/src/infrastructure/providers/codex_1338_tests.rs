// Issue #1338: sub-agent note delivery on the Responses path. `openai-oauth`
// routes here rather than to `openai.rs`, and system messages fold into
// `instructions`, so a note injected as a system message never entered
// `input`. Split from codex_tests.rs for the 750-line limit.

// Nested under `codex_tests`, so `super::super` is the codex module itself.
use super::super::*;

/// `openai-oauth` routes here, not to `openai.rs` (`agent_provider.rs`
/// build_single_provider). A sub-agent note as a system message folded into
/// `instructions` and never entered `input`, so the request ended on the
/// parent's own assistant turn and the model just continued its previous
/// answer. As a user turn it must reach `input` as the trailing entry (#1338).
#[test]
fn test_subagent_note_reaches_input_as_trailing_user_turn() {
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
    let (instructions, input) = CodexProvider::build_input(&messages);
    assert_eq!(
        instructions.as_deref(),
        Some("REAL SYSTEM PROMPT"),
        "the note must not be folded into the instructions blob"
    );
    let last = input.last().expect("input must not be empty");
    assert_eq!(
        last["role"], "user",
        "the request must end on a user turn so the model answers the note"
    );
    assert!(
        last["content"]
            .as_str()
            .unwrap_or_default()
            .contains("subagent_notification"),
        "the note itself must be the trailing turn, got: {last}"
    );
}

#[test]
fn codex_sse_caps_reasoning_delta_accumulation() {
    let mut acc = codex_sse_state::SseAccumulator::default();
    let exact = "a".repeat(8 * 1024 * 1024);
    acc.handle_event(
        &serde_json::json!({"type":"response.reasoning_summary_text.delta","delta": exact}),
    );
    acc.handle_event(
        &serde_json::json!({"type":"response.reasoning_summary_text.delta","delta":"b"}),
    );
    assert_eq!(acc.reasoning.len(), 8 * 1024 * 1024);
}
