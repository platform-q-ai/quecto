//! #2414 review L7: the loop's feedback is always its own user message
//! (a message already sent is never edited), so a chat-completions request
//! can carry consecutive user messages: the prompt, then the feedback. They
//! go on the wire as they are, each its own entry, verbatim and in order:
//! joining them would rewrite the earlier entry, which the previous
//! request ended on, and miss the provider's prompt cache there. The chat
//! format allows them; a local server whose chat template demands strict
//! user/assistant alternation would refuse such a request (a documented
//! limit, #2414).

use super::OpenAiProvider;
use crate::application::providers::ports::ChatRequest;
use crate::domain::message::Message;

#[test]
fn consecutive_user_messages_go_on_the_wire_as_they_are() {
    let messages = vec![
        Message::system("system"),
        Message::user("the prompt"),
        Message::user("feedback: answer concisely"),
    ];
    let request = ChatRequest {
        trace: None,
        admission: None,
        messages: &messages,
        tools: &[],
        model: "accounts/fireworks/models/glm-5p3",
        max_tokens: 256,
        temperature: 0.2,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let body = OpenAiProvider::build_chat_completions_body_for_test("fireworks", &request);
    assert_eq!(
        body["messages"],
        serde_json::json!([
            {"role": "system", "content": "system"},
            {"role": "user", "content": "the prompt"},
            {"role": "user", "content": "feedback: answer concisely"},
        ])
    );
}
