// Issue #1066: the effort vocabulary gained the OpenAI-documented levels
// `none` and `xhigh`. Anthropic models' effort behaviour must stay unchanged:
// Anthropic's own documented vocabulary (low, medium, high, max) is
// transmitted verbatim, and the OpenAI-only levels clamp to the nearest
// documented Anthropic value instead of leaking undocumented strings into
// the request.

use super::*;
use crate::domain::message::Message;
use crate::domain::provider::{ChatRequest, EffortLevel};

fn body_with_effort(effort: Option<EffortLevel>) -> serde_json::Value {
    let messages = vec![Message::user("Hi")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-opus-4-6",
        max_tokens: 4_096,
        temperature: 1.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort,
    };
    let (_sys, body) = AnthropicProvider::build_request_body_public(&req);
    body
}

/// Anthropic's pre-existing vocabulary is transmitted verbatim — unchanged
/// from before #1066.
#[test]
fn anthropic_documented_efforts_transmitted_verbatim_1066() {
    for (level, expected) in [
        (EffortLevel::Low, "low"),
        (EffortLevel::Medium, "medium"),
        (EffortLevel::High, "high"),
        (EffortLevel::Max, "max"),
    ] {
        let body = body_with_effort(Some(level));
        assert_eq!(
            body["output_config"]["effort"], expected,
            "Anthropic effort '{expected}' must be unchanged by #1066, got: {body}"
        );
    }
}

/// The OpenAI-only levels clamp to Anthropic's nearest documented value —
/// undocumented strings must never reach an Anthropic request.
#[test]
fn anthropic_clamps_openai_only_efforts_1066() {
    let body = body_with_effort(Some(EffortLevel::None));
    assert_eq!(
        body["output_config"]["effort"], "low",
        "'none' is not in Anthropic's vocabulary; it must clamp to 'low', got: {body}"
    );
    let body = body_with_effort(Some(EffortLevel::XHigh));
    assert_eq!(
        body["output_config"]["effort"], "high",
        "'xhigh' is not in Anthropic's vocabulary; it must clamp to 'high', got: {body}"
    );
}
