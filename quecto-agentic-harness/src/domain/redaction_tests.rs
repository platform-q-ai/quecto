use super::*;

#[test]
fn scrubs_named_and_prefixed_secrets() {
    assert_eq!(
        redact_secrets("Authorization: Bearer abc.def"),
        "Authorization: [REDACTED]"
    );
    assert_eq!(redact_secrets("sk-livedeadbeef0001"), "[REDACTED]");
    assert_eq!(redact_secrets("token=hunter2"), "[REDACTED]");
    assert_eq!(redact_secrets("--api-key=topsecret"), "--[REDACTED]");
}

#[test]
fn scrubs_positional_provider_tokens() {
    assert_eq!(redact_secrets("AKIAIOSFODNN7EXAMPLE"), "[REDACTED]");
    assert_eq!(
        redact_secrets("ghp_0123456789abcdef0123456789abcdef0123"),
        "[REDACTED]"
    );
    assert_eq!(redact_secrets("xoxb-0123456789-abcdefghij"), "[REDACTED]");
}

#[test]
fn preserves_non_secret_text() {
    assert_eq!(redact_secrets("ls -la /tmp"), "ls -la /tmp");
    assert_eq!(redact_secrets("usage_limit_reached"), "usage_limit_reached");
}
