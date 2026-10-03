//! #2435: only the known shapes of a provider refusing a model for the
//! account or auth mode in use are definitive refusals.

use super::*;

fn provider(message: &str) -> DomainError {
    DomainError::Provider(message.to_string())
}

#[test]
fn codex_refusing_a_model_for_a_chatgpt_account_is_a_refusal_with_its_detail() {
    let err = provider(
        r#"HTTP 400 from Codex: {"detail":"The 'gpt-5.5-mini' model is not supported when using Codex with a ChatGPT account."}"#,
    );
    assert_eq!(
        model_refusal(&err).as_deref(),
        Some("The 'gpt-5.5-mini' model is not supported when using Codex with a ChatGPT account.")
    );
}

#[test]
fn openai_and_anthropic_model_not_found_replies_are_refusals() {
    let openai = provider(
        r#"HTTP 404 from OpenAI: {
    "error": {
        "message": "The model `gpt-9` does not exist or you do not have access to it.",
        "type": "invalid_request_error",
        "param": null,
        "code": "model_not_found"
    }
}"#,
    );
    assert_eq!(
        model_refusal(&openai).as_deref(),
        Some("The model `gpt-9` does not exist or you do not have access to it.")
    );
    let anthropic = provider(
        r#"HTTP 404 from Anthropic: {"type":"error","error":{"type":"not_found_error","message":"model: claude-opus-4-1"}}"#,
    );
    assert_eq!(
        model_refusal(&anthropic).as_deref(),
        Some("model: claude-opus-4-1")
    );
}

#[test]
fn a_refusal_whose_body_is_not_json_keeps_the_body() {
    let err = provider(
        "HTTP 400 from Codex: The 'x' model is not supported when using Codex with a ChatGPT account.",
    );
    assert_eq!(
        model_refusal(&err).as_deref(),
        Some("The 'x' model is not supported when using Codex with a ChatGPT account.")
    );
}

#[test]
fn every_other_error_is_not_a_refusal() {
    for message in [
        // A malformed request: a 400 that says nothing of the model.
        r#"HTTP 400 from Codex: {"detail":"Unsupported parameter: temperature"}"#,
        // The refusal phrase under another status is not the known shape.
        r#"HTTP 500 from Codex: {"detail":"The 'm' model is not supported when using Codex with a ChatGPT account."}"#,
        // A credential failure is re-authenticated, never a refusal.
        r#"HTTP 401 from OpenAI: {"error":{"code":"invalid_api_key"}}"#,
        // A 404 for something other than the model.
        r#"HTTP 404 from Anthropic: {"type":"error","error":{"type":"not_found_error","message":"file: f-1"}}"#,
        r#"HTTP 404 from OpenAI: {"error":{"code":"not_found"}}"#,
        // No status at all.
        "model_not_found",
        "",
    ] {
        assert_eq!(model_refusal(&provider(message)), None, "{message}");
    }
    assert_eq!(
        model_refusal(&DomainError::Tool("model_not_found".into())),
        None
    );
}

#[test]
fn a_long_reason_is_bounded() {
    let detail = format!(
        "The 'm' model is not supported when using Codex with a ChatGPT account.{}",
        "x".repeat(2_000)
    );
    let err = provider(&format!(
        "HTTP 400 from Codex: {}",
        serde_json::json!({ "detail": detail })
    ));
    let reason = model_refusal(&err).expect("a refusal");
    assert!(
        reason.len() <= MODEL_REFUSAL_REASON_MAX_BYTES,
        "{}",
        reason.len()
    );
    assert!(reason.starts_with("The 'm' model is not supported"));
}

/// A refusal relayed as a stream error carries the `provider error: `
/// prefix of the error's display; it is the same refusal.
#[test]
fn a_refusal_relayed_through_a_stream_error_is_still_a_refusal() {
    let err = provider(
        r#"provider error: HTTP 400 from Codex: {"detail":"The 'm' model is not supported when using Codex with a ChatGPT account."}"#,
    );
    assert!(model_refusal(&err).is_some());
}
