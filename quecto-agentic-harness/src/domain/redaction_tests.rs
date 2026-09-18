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

#[test]
fn url_userinfo_is_redacted_wherever_a_url_appears() {
    assert_eq!(
        redact_url_userinfo("--repo https://user:ghp_secret@host/x/y is unreachable"),
        "--repo https://***@host/x/y is unreachable"
    );
    assert_eq!(
        redact_url_userinfo("ssh://git@example.test:2222/r.git and https://t0k3n@h/p"),
        "ssh://***@example.test:2222/r.git and https://***@h/p"
    );
    for untouched in [
        "https://host/x/y",
        "git@github.com:org/repo.git",
        "https://host/path?mail=a@b",
        "",
    ] {
        assert_eq!(redact_url_userinfo(untouched), untouched);
    }
}

#[test]
fn an_unencoded_at_in_the_password_is_redacted_to_the_path() {
    assert_eq!(
        redact_url_userinfo("https://u:p@ss@host/x/y"),
        "https://***@host/x/y"
    );
}
