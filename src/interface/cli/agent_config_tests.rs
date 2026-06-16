// Issue #300: --config flag tests + build_agent_provider tests

use super::*;
use crate::domain::message::Message;
use crate::domain::provider::ChatRequest;
use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};
use crate::infrastructure::config::Config;
use crate::interface::cli::run_with_output;

/// Helper to load a Config from a JSON string via a temp file.
fn config_from_str(json: &str) -> Config {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), json).unwrap();
    Config::load(tmp.path().to_str().unwrap()).unwrap()
}

fn openai_oauth_jwt(account_id: &str) -> String {
    use base64::Engine;
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!(
        r#"{{"https://api.openai.com/auth":{{"chatgpt_account_id":"{}"}}}}"#,
        account_id
    ));
    format!("{}.{}.sig", header, payload)
}

fn chat_request<'a>(messages: &'a [Message], model: &'a str) -> ChatRequest<'a> {
    ChatRequest {
        messages,
        tools: &[],
        model,
        max_tokens: 128,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
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
fn test_build_agent_provider_openai_compatible_ignores_openai_oauth() {
    let tmp = tempfile::TempDir::new().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(wiremock::MockServer::start());
    rt.block_on(async {
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/chat/completions"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer sk-spark",
            ))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": "chatcmpl-test",
                    "object": "chat.completion",
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": "Spark endpoint used"},
                        "finish_reason": "stop"
                    }],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
                })),
            )
            .mount(&server)
            .await;
    });

    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "openai".to_string(),
            token: openai_oauth_jwt("acct_test"),
            method: AuthMethod::OAuth,
            expires_at: Some(4_102_444_800),
            refresh_token: Some("refresh".to_string()),
            account_id: Some("acct_test".to_string()),
        })
        .unwrap();

    let config = config_from_str(&format!(
        r#"{{"providers":{{"openai":{{"api_key":"sk-openai-config"}},"openai_compatible":{{"endpoints":[{{"prefix":"spark","api_key":"sk-spark","api_base":"{}/v1"}}]}}}}}}"#,
        server.uri()
    ));
    let provider = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap();
    let messages = vec![Message::user("Hi")];
    let response = rt
        .block_on(provider.chat(chat_request(&messages, "spark/qwen3")))
        .unwrap();
    assert_eq!(response.content.as_deref(), Some("Spark endpoint used"));
}

#[test]
fn test_build_agent_provider_disable_codex_routing_prefers_config_key() {
    let tmp = tempfile::TempDir::new().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(wiremock::MockServer::start());
    rt.block_on(async {
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/chat/completions"))
            .and(wiremock::matchers::header(
                "authorization",
                "Bearer sk-from-config",
            ))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "id": "chatcmpl-test",
                    "object": "chat.completion",
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": "OpenAI slot used config key"},
                        "finish_reason": "stop"
                    }],
                    "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
                })),
            )
            .mount(&server)
            .await;
    });

    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "openai".to_string(),
            token: openai_oauth_jwt("acct_test"),
            method: AuthMethod::OAuth,
            expires_at: Some(4_102_444_800),
            refresh_token: Some("refresh".to_string()),
            account_id: Some("acct_test".to_string()),
        })
        .unwrap();

    let config = config_from_str(&format!(
        r#"{{"providers":{{"openai":{{"api_key":"sk-from-config","api_base":"{}/v1","disable_codex_routing":true}}}}}}"#,
        server.uri()
    ));
    let provider = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap();
    let messages = vec![Message::user("Hi")];
    let response = rt
        .block_on(provider.chat(chat_request(&messages, "openai/custom-model")))
        .unwrap();
    assert_eq!(
        response.content.as_deref(),
        Some("OpenAI slot used config key")
    );
}

#[test]
fn test_build_agent_provider_rejects_duplicate_openai_compatible_prefixes() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = config_from_str(
        r#"{"providers":{"openai_compatible":{"endpoints":[
            {"prefix":"spark","api_key":"sk-one","api_base":"http://127.0.0.1:8000/v1"},
            {"prefix":"SPARK","api_key":"sk-two","api_base":"http://127.0.0.1:8001/v1"}
        ]}}}"#,
    );
    let result = build_agent_provider(&config, tmp.path(), &reqwest::Client::new());
    assert!(result.unwrap_err().contains("duplicate openai_compatible"));
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
