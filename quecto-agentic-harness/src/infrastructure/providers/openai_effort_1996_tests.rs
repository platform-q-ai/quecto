//! #1996: the chat-completions request builder serialises a selected effort
//! as `reasoning_effort` for OpenAI-compatible endpoints that accept it,
//! omits it when none is selected, never pairs it with Fireworks' mutually
//! exclusive `thinking` parameter, and never sends it to OpenAI's own Chat
//! Completions endpoint (which rejects it with function tools). The builder
//! receives only levels the application already admitted for the model, so
//! it transmits verbatim and decides nothing about the model.

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

fn body(provider: &str, request: &ChatRequest<'_>) -> serde_json::Value {
    OpenAiProvider::build_chat_completions_body_for_test(provider, request)
}

#[test]
fn a_selected_effort_reaches_the_wire_as_reasoning_effort() {
    let messages = vec![Message::user("hi")];
    for (provider, model, level, expected) in [
        ("xai", "grok-4.6", EffortLevel::XHigh, "xhigh"),
        ("xai", "grok-4.5", EffortLevel::High, "high"),
        (
            "fireworks",
            "accounts/fireworks/models/glm-5p3",
            EffortLevel::Low,
            "low",
        ),
    ] {
        let body = body(provider, &request(&messages, model, Some(level)));
        assert_eq!(body["reasoning_effort"], expected, "{model}: {body}");
        assert!(body.get("thinking").is_none(), "{model}: never both");
    }
}

#[test]
fn no_selected_effort_sends_no_reasoning_option() {
    let messages = vec![Message::user("hi")];
    let body = body(
        "spark-local",
        &request(&messages, "qwen3.6-35b-a3b-int4", None),
    );
    assert!(body.get("reasoning_effort").is_none(), "{body}");
    assert!(body.get("thinking").is_none(), "{body}");
}

#[test]
fn openais_own_chat_completions_endpoint_never_transmits_reasoning_effort() {
    // OpenAI rejects reasoning_effort with function tools on Chat
    // Completions; its reasoning ids go to the Responses API, and the OAuth
    // fallback without an account id lands here.
    let messages = vec![Message::user("hi")];
    let body = body(
        "openai",
        &request(&messages, "gpt-5.5", Some(EffortLevel::High)),
    );
    assert!(body.get("reasoning_effort").is_none(), "{body}");
}

#[test]
fn changing_effort_between_turns_changes_the_next_request() {
    let messages = vec![Message::user("hi")];
    let first = body(
        "xai",
        &request(&messages, "grok-4.6", Some(EffortLevel::Low)),
    );
    let second = body(
        "xai",
        &request(&messages, "grok-4.6", Some(EffortLevel::High)),
    );
    assert_eq!(first["reasoning_effort"], "low");
    assert_eq!(second["reasoning_effort"], "high");
}
