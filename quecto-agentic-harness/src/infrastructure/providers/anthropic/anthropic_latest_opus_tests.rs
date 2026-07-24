use super::*;
use crate::domain::message::Message;
use crate::domain::provider::ChatRequest;

#[test]
fn test_opus_47_and_48_omit_deprecated_temperature() {
    for model in ["claude-opus-4-7", "claude-opus-4-8"] {
        let messages = vec![Message::user("Hi")];
        let req = ChatRequest {
            messages: &messages,
            tools: &[],
            model,
            max_tokens: 4_096,
            temperature: 0.7,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
            effort: None,
        };
        let (_sys, body) = AnthropicProvider::build_request_body_public(&req);
        assert_eq!(
            body["thinking"]["type"], "adaptive",
            "model {model}: {body}"
        );
        assert!(
            body.get("temperature").is_none(),
            "temperature is deprecated for {model} and must be omitted: {body}"
        );
        assert_eq!(
            body["output_config"]["effort"], "low",
            "model {model}: {body}"
        );
    }
}

#[test]
fn test_opus_5_omits_deprecated_temperature() {
    let messages = vec![Message::user("Hi")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-opus-5",
        max_tokens: 4_096,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body_public(&req);
    assert_eq!(body["thinking"]["type"], "adaptive", "{body}");
    assert!(
        body.get("temperature").is_none(),
        "temperature is deprecated for claude-opus-5 and must be omitted: {body}"
    );
}

#[test]
fn test_opus_5_omits_interleaved_thinking_beta() {
    let beta = AnthropicProvider::build_beta_header_public("claude-opus-5", false);
    assert!(
        !beta.contains("interleaved-thinking-2025-05-14"),
        "claude-opus-5 has built-in interleaved thinking; beta should be omitted: {beta}"
    );
}

#[test]
fn test_opus_47_and_48_keep_interleaved_thinking_beta() {
    for model in ["claude-opus-4-7", "claude-opus-4-8"] {
        let beta = AnthropicProvider::build_beta_header_public(model, false);
        assert!(
            beta.contains("interleaved-thinking-2025-05-14"),
            "{model} still needs interleaved-thinking beta unless docs prove otherwise: {beta}"
        );
    }
}
