use super::CredentialEnv;

#[test]
fn a_credential_value_never_reaches_debug_output() {
    let credential = CredentialEnv {
        name: "ANTHROPIC_API_KEY".into(),
        value: "secret-value-123".into(),
    };
    let shown = format!("{credential:?}");
    assert!(shown.contains("ANTHROPIC_API_KEY"), "{shown}");
    assert!(!shown.contains("secret-value-123"), "{shown}");
}
