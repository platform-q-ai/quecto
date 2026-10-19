//! `quecto status` through the composed inspection (#1843): configured
//! keys of every provider the configuration names, and the credentials
//! the auth capability stores, word for word.
use crate::domain::auth::credential::{AuthMethod, Credential};
use crate::infrastructure::auth::credential_store::CredentialStore;
use crate::interface::cli::{CliContext, run_with_output};

fn status(base: &std::path::Path) -> crate::interface::cli::CliOutput {
    let ctx = CliContext {
        base_dir: Some(base.to_path_buf()),
        configuration: Some(crate::composition::configuration::build_configuration_handles),
        ..Default::default()
    };
    run_with_output(["quecto", "status"].map(String::from).to_vec(), &ctx)
}

/// The lines after `Effort:`, where provider presence is reported.
fn presence_lines(stdout: &str) -> Vec<&str> {
    stdout
        .lines()
        .skip_while(|line| !line.starts_with("  Effort:"))
        .skip(1)
        .collect()
}

#[test]
fn status_reports_stored_credentials_beside_the_configured_keys() {
    let base = tempfile::TempDir::new().unwrap();
    std::fs::write(
        base.path().join("config.json"),
        r#"{"providers": {"anthropic": {"api_key": "fake-ant"}}}"#,
    )
    .unwrap();
    let store = CredentialStore::new(base.path());
    for (provider, method, expires_at) in [
        ("xai", AuthMethod::OAuth, Some(0)),
        ("openai", AuthMethod::OAuth, None),
    ] {
        store
            .store(Credential {
                provider: provider.to_string(),
                token: "fake-token".to_string(),
                method,
                expires_at,
                refresh_token: None,
                account_id: None,
            })
            .unwrap();
    }
    let out = status(base.path());
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        presence_lines(&out.stdout),
        [
            "  OpenAI API:    not set",
            "  Anthropic API: configured",
            "  Credentials:   openai (oauth) — active, xai (oauth) — expired",
        ]
    );
    assert!(!out.stdout.contains("fake-"), "{}", out.stdout);
}

#[test]
fn status_names_each_openai_compatible_endpoint_with_a_key() {
    let base = tempfile::TempDir::new().unwrap();
    std::fs::write(
        base.path().join("config.json"),
        r#"{"providers": {"openai_compatible": {"endpoints": [
            {"prefix": "groq", "api_key": "fake-g", "api_base": "https://g.test/v1"},
            {"prefix": "local", "api_key": "", "api_base": "http://127.0.0.1:9/v1"},
            {"prefix": "together", "api_key": "fake-t", "api_base": "https://t.test/v1"}
        ]}}}"#,
    )
    .unwrap();
    let out = status(base.path());
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        presence_lines(&out.stdout),
        [
            "  OpenAI API:    not set",
            "  Anthropic API: not set",
            "  Endpoint keys: groq, together",
            "  Credentials:   none stored",
        ]
    );
}

#[test]
fn a_malformed_credential_store_is_reported_in_fixed_words_not_fatal() {
    let base = tempfile::TempDir::new().unwrap();
    std::fs::write(
        base.path().join("credentials.json"),
        r#"{"credentials":{"openai":{"provider":"openai","token":12345678,"method":"token","expires_at":"sk-FAKE-SECRET-IN-WRONG-FIELD"}}}"#,
    )
    .unwrap();
    let out = status(base.path());
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        *presence_lines(&out.stdout).last().unwrap(),
        "  Credentials:   unreadable (malformed store)"
    );
    let all = format!("{}{}", out.stdout, out.stderr);
    for secret in ["12345678", "sk-FAKE-SECRET-IN-WRONG-FIELD", "config error"] {
        assert!(!all.contains(secret), "{secret} reached status: {all}");
    }
}

#[test]
fn a_credential_store_that_cannot_be_read_says_so() {
    let base = tempfile::TempDir::new().unwrap();
    std::fs::create_dir(base.path().join("credentials.json")).unwrap();
    let out = status(base.path());
    assert_eq!(out.exit_code, 0, "stderr: {}", out.stderr);
    assert_eq!(
        *presence_lines(&out.stdout).last().unwrap(),
        "  Credentials:   unreadable"
    );
}

#[test]
fn status_without_composed_configuration_refuses() {
    let out = run_with_output(
        ["quecto", "status"].map(String::from).to_vec(),
        &CliContext::default(),
    );
    assert_eq!(out.exit_code, 1);
    assert_eq!(out.stderr, "configuration capability not composed\n");
}
