//! #1679 P2 AC5: typed body/event classification, never human error prose.
use quecto::infrastructure::providers::admission_feedback::is_typed_throttle;
use serde_json::json;

#[test]
fn rate_limit_error_type_is_throttle() {
    assert!(is_typed_throttle(
        &json!({"error":{"type":"rate_limit_error"}})
    ));
}
#[test]
fn overloaded_error_type_is_throttle() {
    assert!(is_typed_throttle(
        &json!({"error":{"type":"overloaded_error"}})
    ));
}
#[test]
fn responses_nested_failure_is_throttle() {
    assert!(is_typed_throttle(
        &json!({"type":"response.failed","response":{"error":{"code":"rate_limit_exceeded"}}})
    ));
}
#[test]
fn billing_code_overrides_rate_limit_type() {
    assert!(!is_typed_throttle(
        &json!({"error":{"type":"rate_limit_error","code":"insufficient_quota"}})
    ));
}
#[test]
fn prose_alone_is_not_a_throttle() {
    assert!(!is_typed_throttle(
        &json!({"error":{"message":"429 rate_limit_error overloaded_error"}})
    ));
}
#[test]
fn authentication_error_is_terminal() {
    assert!(!is_typed_throttle(
        &json!({"error":{"type":"authentication_error"}})
    ));
}
#[test]
fn client_error_is_terminal() {
    assert!(!is_typed_throttle(
        &json!({"error":{"type":"invalid_request_error"}})
    ));
}
