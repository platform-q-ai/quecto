// Issue #300: --config flag tests + build_agent_provider tests

use super::*;
use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};
use crate::infrastructure::config::Config;
use crate::interface::cli::run_with_output;

/// Helper to load a Config from a JSON string via a temp file.
fn config_from_str(json: &str) -> Config {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), json).unwrap();
    Config::load(tmp.path().to_str().unwrap()).unwrap()
}

// ===================================================================
// build_agent_provider() tests
// ===================================================================

#[test]
fn test_build_agent_provider_openai_only() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = config_from_str(r#"{"providers":{"openai":{"api_key":"sk-test-key"}}}"#);
    let result = build_agent_provider(&config, tmp.path(), &reqwest::Client::new());
    assert!(result.is_ok());
}

#[test]
fn test_build_agent_provider_anthropic_only() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = config_from_str(r#"{"providers":{"anthropic":{"api_key":"sk-ant-test-key"}}}"#);
    let result = build_agent_provider(&config, tmp.path(), &reqwest::Client::new());
    assert!(result.is_ok());
}

#[test]
fn test_build_agent_provider_both_providers() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = config_from_str(
        r#"{"providers":{"openai":{"api_key":"sk-test"},"anthropic":{"api_key":"sk-ant-test"}}}"#,
    );
    let result = build_agent_provider(&config, tmp.path(), &reqwest::Client::new());
    assert!(result.is_ok());
}

#[test]
fn test_build_agent_provider_no_keys() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config =
        config_from_str(r#"{"providers":{"openai":{"api_key":""},"anthropic":{"api_key":""}}}"#);
    let result = build_agent_provider(&config, tmp.path(), &reqwest::Client::new());
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("no LLM providers"));
}

#[test]
fn test_build_agent_provider_rejects_unapproved_api_base_host() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = config_from_str(
        r#"{"providers":{"openai":{"api_key":"sk-test","api_base":"https://custom.openai.com/v1"}}}"#,
    );
    let result = build_agent_provider(&config, tmp.path(), &reqwest::Client::new());
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .contains("openai provider configuration error")
    );
}

#[test]
fn test_build_agent_provider_with_credential_store() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "openai".to_string(),
            token: "sk-stored-cred".to_string(),
            method: AuthMethod::Token,
            expires_at: None,
            refresh_token: None,
            account_id: None,
        })
        .unwrap();

    let config = config_from_str(r#"{"providers":{"openai":{"api_key":""}}}"#);
    let result = build_agent_provider(&config, tmp.path(), &reqwest::Client::new());
    assert!(result.is_ok());
}

#[test]
fn test_agent_config_flag_loads_custom_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = tmp.path().join("custom.json");
    std::fs::write(&cfg, "{}").unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().into()),
        config_path: Some(cfg.clone()),
        ..Default::default()
    };
    let args = vec![
        "quecto".into(),
        "agent".into(),
        "--config".into(),
        cfg.to_str().unwrap().into(),
        "-m".into(),
        "Hi".into(),
    ];
    let out = run_with_output(args, &ctx);
    assert!(
        !out.stderr.contains("config not found"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn test_agent_config_flag_missing_value() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(tmp.path().join("config.json"), "{}").unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().into()),
        ..Default::default()
    };
    let out = run_with_output(
        vec!["quecto".into(), "agent".into(), "--config".into()],
        &ctx,
    );
    assert_eq!(out.exit_code, 1);
    assert!(out.stderr.contains("--config requires"));
}

#[test]
fn test_agent_config_flag_nonexistent_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().into()),
        ..Default::default()
    };
    let args = vec![
        "quecto".into(),
        "agent".into(),
        "--config".into(),
        "/tmp/no-such-config.json".into(),
        "-m".into(),
        "hi".into(),
    ];
    let out = run_with_output(args, &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("config not found"),
        "stderr: {}",
        out.stderr
    );
}
