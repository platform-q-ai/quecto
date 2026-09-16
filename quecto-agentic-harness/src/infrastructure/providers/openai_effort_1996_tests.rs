//! #1996: the chat-completions request builder serialises a selected effort
//! as `reasoning_effort`, omits it when none is selected, and never pairs it
//! with Fireworks' mutually exclusive `thinking` parameter. The builder
//! receives only levels the application already admitted for the model, so
//! it transmits verbatim and decides nothing.

use super::OpenAiProvider;
use crate::application::providers::ports::ChatRequest;
use crate::domain::message::Message;
use crate::domain::provider::EffortLevel;

fn request<'a>(
    messages: &'a [Message],
    model: &'a str,
    effort: Option<EffortLevel>,
) -> ChatRequest<'a> {
    ChatRequest {
        trace: None,
        admission: None,
        messages,
        tools: &[],
        model,
        max_tokens: 256,
        temperature: 0.2,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort,
    }
}

#[test]
fn a_selected_effort_reaches_the_wire_as_reasoning_effort() {
    let messages = vec![Message::user("hi")];
    for (model, level, expected) in [
        ("grok-4.6", EffortLevel::XHigh, "xhigh"),
        ("grok-4.5", EffortLevel::High, "high"),
        ("accounts/fireworks/models/glm-5p3", EffortLevel::Low, "low"),
        ("gpt-5.6-sol", EffortLevel::None, "none"),
    ] {
        let body = OpenAiProvider::build_request_body(&request(&messages, model, Some(level)));
        assert_eq!(body["reasoning_effort"], expected, "{model}: {body}");
        assert!(body.get("thinking").is_none(), "{model}: never both");
    }
}

#[test]
fn no_selected_effort_sends_no_reasoning_option() {
    let messages = vec![Message::user("hi")];
    let body =
        OpenAiProvider::build_request_body(&request(&messages, "qwen3.6-35b-a3b-int4", None));
    assert!(body.get("reasoning_effort").is_none(), "{body}");
    assert!(body.get("thinking").is_none(), "{body}");
}

#[test]
fn changing_effort_between_turns_changes_the_next_request() {
    let messages = vec![Message::user("hi")];
    let first =
        OpenAiProvider::build_request_body(&request(&messages, "grok-4.6", Some(EffortLevel::Low)));
    let second = OpenAiProvider::build_request_body(&request(
        &messages,
        "grok-4.6",
        Some(EffortLevel::High),
    ));
    assert_eq!(first["reasoning_effort"], "low");
    assert_eq!(second["reasoning_effort"], "high");
}
