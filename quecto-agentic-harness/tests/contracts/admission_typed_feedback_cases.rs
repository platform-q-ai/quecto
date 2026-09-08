//! Typed SSE/HTTP error precedence boundaries; no error-message matching.
use quecto::infrastructure::providers::admission_feedback::is_typed_throttle;
use serde_json::json;

#[test]
fn direct_sse_error_envelope_is_recognized() {
    assert!(is_typed_throttle(
        &json!({"type":"error","error":{"code":"rate_limit_exceeded"}})
    ));
}
#[test]
fn terminal_billing_response_code_overrides_overload() {
    assert!(!is_typed_throttle(
        &json!({"response":{"error":{"type":"overloaded_error","code":"billing_hard_limit_reached"}}})
    ));
}
#[test]
fn success_payload_with_throttle_words_is_not_feedback() {
    assert!(!is_typed_throttle(
        &json!({"type":"response.output_text.delta","delta":"rate_limit_exceeded"})
    ));
}
#[test]
fn missing_error_body_is_not_typed_confirmation() {
    assert!(!is_typed_throttle(&json!(null)));
}
