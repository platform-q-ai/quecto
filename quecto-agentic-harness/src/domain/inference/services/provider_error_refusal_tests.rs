//! Finite compatibility markers only; raw payloads have no refusal authority.
use super::*;

fn provider(message: &str) -> DomainError {
    DomainError::Provider(message.to_string())
}

#[test]
fn codex_refusing_a_model_has_fixed_guidance() {
    let err = provider("HTTP 400 from Codex: class=client; refusal=model");
    let reason = model_refusal(&err).expect("known refusal");
    assert_eq!(
        reason,
        "This model is unavailable for this account. Choose an available model or check account access."
    );
    assert!(reason.len() <= MODEL_REFUSAL_REASON_MAX_BYTES);
}

#[test]
fn openai_and_anthropic_model_not_found_have_fixed_guidance() {
    for vendor in ["OpenAI", "Anthropic"] {
        let reason = model_refusal(&provider(&format!(
            "HTTP 404 from {vendor}: class=client; refusal=model"
        )))
        .expect("known refusal");
        assert_eq!(
            reason,
            "This model is unavailable for this account. Choose an available model or check account access."
        );
        assert!(reason.len() <= MODEL_REFUSAL_REASON_MAX_BYTES);
    }
}

#[test]
fn raw_json_and_arbitrary_prose_have_no_refusal_authority() {
    for message in [
        r#"HTTP 400 from Codex: {"detail":"The 'm' model is not supported when using Codex with a ChatGPT account."}"#,
        "HTTP 400 from Codex: The 'x' model is not supported when using Codex with a ChatGPT account.",
        r#"HTTP 404 from OpenAI: {"error":{"code":"model_not_found"}}"#,
        r#"HTTP 404 from Anthropic: {"error":{"type":"not_found_error","message":"model: m"}}"#,
        "model_not_found",
        "",
    ] {
        assert_eq!(model_refusal(&provider(message)), None);
    }
    assert_eq!(
        model_refusal(&DomainError::Tool("model_not_found".into())),
        None
    );
}

#[test]
fn finite_refusal_grammar_rejects_wrong_status_vendor_and_suffix() {
    for message in [
        "HTTP 500 from Codex: class=server; refusal=model",
        "HTTP 401 from OpenAI: class=auth; refusal=model",
        "HTTP 404 from Unknown: class=client; refusal=model",
        "HTTP 404 from OpenAI: class=client; refusal=model secret",
        "HTTP 404 from OpenAI: class=client; refusal=file",
        "HTTP 404 from OpenAI: class=unknown; refusal=model",
        "HTTP 400 from Codex: class=client; refusal=encrypted",
    ] {
        assert_eq!(model_refusal(&provider(message)), None, "{message}");
    }
}

#[test]
fn a_long_raw_reason_cannot_be_persisted() {
    let err = provider(&format!(
        "HTTP 400 from Codex: The 'm' model is not supported when using Codex with a ChatGPT account.{}",
        "x".repeat(2_000)
    ));
    assert_eq!(model_refusal(&err), None);
}

#[test]
fn a_refusal_relayed_through_a_stream_error_is_still_a_refusal() {
    assert!(
        model_refusal(&provider(
            "provider error: HTTP 400 from Codex: class=client; refusal=model"
        ))
        .is_some()
    );
}

#[test]
fn model_refusal_local_guidance_does_not_admit_arbitrary_suffixes() {
    for suffix in [
        " arbitrary provider prose",
        "\n\nModel unavailable: secret-canary",
        "\n\nBilling/quota: secret-canary",
    ] {
        let error = DomainError::Provider(format!(
            "HTTP 400 from Codex: class=client; refusal=model{suffix}"
        ));
        assert_eq!(model_refusal(&error), None);
    }
}
