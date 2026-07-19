// Issue #1066: reasoning-effort behaviour of the Responses request body.
// Split from codex_tests.rs for the 750-line limit.

use super::*;
use crate::domain::provider::EffortLevel;

/// Issue #1066: when no effort is configured, the request must omit
/// `reasoning.effort` entirely so OpenAI's server default applies — the
/// kernel must not invent a "medium" fallback.
#[test]
fn test_build_request_body_omits_effort_when_unconfigured_1066() {
    let messages = vec![Message::system("Be concise."), Message::user("Hi")];
    let tools = vec![];
    let request = ChatRequest {
        messages: &messages,
        tools: &tools,
        model: "gpt-5.6-sol",
        max_tokens: 4096,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let body = CodexProvider::build_request_body(&request);
    assert!(
        body["reasoning"].get("effort").is_none(),
        "unconfigured effort must be omitted so the server default applies (#1066), got {:?}",
        body["reasoning"]["effort"]
    );
}

/// Issue #1066: every OpenAI-documented effort level (none, low, medium,
/// high, xhigh) must be transmitted verbatim on the Responses API.
#[test]
fn test_build_request_body_transmits_openai_documented_efforts_1066() {
    for level in ["none", "low", "medium", "high", "xhigh"] {
        let effort = EffortLevel::parse(level)
            .expect("OpenAI-documented effort level must be configurable (#1066)");
        let messages = vec![Message::system("Be concise."), Message::user("Hi")];
        let tools = vec![];
        let request = ChatRequest {
            messages: &messages,
            tools: &tools,
            model: "gpt-5.6-sol",
            max_tokens: 4096,
            temperature: 0.7,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
            effort: Some(effort),
        };
        let body = CodexProvider::build_request_body(&request);
        assert_eq!(
            body["reasoning"]["effort"], *level,
            "configured effort '{level}' must be transmitted verbatim (#1066)"
        );
    }
}

/// Review follow-up (#1066): `max` is Anthropic-only vocabulary. OpenAI's
/// documented scale tops out at `xhigh`, so a configured `max` must clamp to
/// `xhigh` on the wire instead of being sent verbatim (which OpenAI rejects).
#[test]
fn test_build_request_body_clamps_max_effort_to_xhigh_1066() {
    let messages = vec![Message::system("Be concise."), Message::user("Hi")];
    let tools = vec![];
    let request = ChatRequest {
        messages: &messages,
        tools: &tools,
        model: "gpt-5.6-sol",
        max_tokens: 4096,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: Some(EffortLevel::Max),
    };
    let body = CodexProvider::build_request_body(&request);
    assert_eq!(
        body["reasoning"]["effort"], "xhigh",
        "effort 'max' is outside OpenAI's documented scale and must clamp to 'xhigh' (#1066)"
    );
    assert_eq!(body["text"]["verbosity"], "high");
}

/// Review follow-up (#1066): with no configured effort, `text.verbosity`
/// must also be omitted — hardcoding "medium" would override OpenAI's
/// server-side default, the same defect class as the removed effort fallback.
#[test]
fn test_build_request_body_omits_verbosity_when_unconfigured_1066() {
    let messages = vec![Message::system("Be concise."), Message::user("Hi")];
    let tools = vec![];
    let request = ChatRequest {
        messages: &messages,
        tools: &tools,
        model: "gpt-5.6-sol",
        max_tokens: 4096,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let body = CodexProvider::build_request_body(&request);
    assert!(
        body.get("text").is_none(),
        "unconfigured effort must omit text.verbosity so the server default applies (#1066), got {:?}",
        body.get("text")
    );
}
