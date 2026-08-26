//! Contract for the application `RefreshRedactionPort` (issue #1574): every
//! implementation strips credential material from human-facing refresh text
//! while preserving the non-secret remainder.

use quecto::application::catalogue_refresh::NoopRedaction;
use quecto::application::ports::RefreshRedactionPort;
use quecto::infrastructure::catalogue_discovery::SecretsRedaction;

fn assert_redaction_contract(port: &dyn RefreshRedactionPort, secrets: &[&str]) {
    let text = format!("401 unauthorized for bearer {}", secrets.join(" and "));
    let redacted = port.redact(&text);
    for secret in secrets {
        assert!(
            !redacted.contains(secret),
            "redaction must strip '{secret}': {redacted}"
        );
    }
    assert!(
        redacted.contains("401 unauthorized"),
        "redaction must preserve the non-secret remainder: {redacted}"
    );
}

#[test]
fn secrets_redaction_satisfies_the_contract_for_every_configured_secret() {
    let port = SecretsRedaction::new(vec![
        "sk-alpha-secret".to_string(),
        "sk-beta-secret".to_string(),
    ]);
    assert_redaction_contract(&port, &["sk-alpha-secret", "sk-beta-secret"]);
}

#[test]
fn secrets_redaction_ignores_degenerate_secret_values() {
    // An empty or one-character "secret" would redact everywhere; the port
    // must leave ordinary text intact rather than shredding it.
    let port = SecretsRedaction::new(vec![String::new(), "x".to_string()]);
    assert_eq!(port.redact("exact text"), "exact text");
}

#[test]
fn noop_redaction_is_identity_for_contexts_without_secrets() {
    assert_eq!(NoopRedaction.redact("plain text"), "plain text");
}
