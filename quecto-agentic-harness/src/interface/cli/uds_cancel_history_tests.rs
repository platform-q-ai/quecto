use super::*;
use crate::domain::message::{Message, ToolCall};

fn assistant_call(id: &str, name: &str) -> Message {
    let mut message = Message::assistant("partial answer", vec![]);
    message.tool_calls = vec![ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: "{}".into(),
    }];
    message
}

#[test]
fn finalize_unknown_prompt_is_empty_and_leaves_history_unchanged() {
    let mut messages = vec![Message::user("hello")];
    let finalized = finalize_interrupted_turn(&mut messages, uuid::Uuid::new_v4());

    assert!(finalized.retained_tail.is_empty());
    assert!(finalized.recordable_messages().is_empty());
    assert_eq!(messages.len(), 1);
}

#[test]
fn finalize_preserves_answered_tool_call_without_synthetic_result() {
    let prompt = Message::user("run it");
    let prompt_id = prompt.id();
    let assistant = assistant_call("call-1", "bash");
    let tool = Message::tool("call-1", "done");
    let mut messages = vec![prompt, assistant, tool];

    let finalized = finalize_interrupted_turn(&mut messages, prompt_id);

    assert_eq!(finalized.retained_tail.len(), 2);
    assert!(finalized.synthetic_results.is_empty());
    assert_eq!(messages.len(), 3);
}

#[test]
fn finalize_synthesizes_error_for_unanswered_tool_call_and_drops_chatter() {
    let prompt = Message::user("run it");
    let prompt_id = prompt.id();
    let assistant = assistant_call("call-2", "read");
    let chatter = Message::assistant("unrelated", vec![]);
    let mut messages = vec![prompt, assistant, chatter];

    let finalized = finalize_interrupted_turn(&mut messages, prompt_id);

    assert_eq!(finalized.retained_tail.len(), 1);
    assert_eq!(finalized.synthetic_results.len(), 1);
    assert!(finalized.synthetic_results[0].is_error);
    assert_eq!(
        finalized.synthetic_results[0].tool_name.as_deref(),
        Some("read")
    );
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[1].content, "");
}
