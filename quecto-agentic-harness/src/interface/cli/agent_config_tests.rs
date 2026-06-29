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
            .contains("openai-api provider configuration error")
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
        .block_on(provider.chat(chat_request(&messages, "openai-api/custom-model")))
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
    // An explicit --config pointing at a missing file must error, not silently
    // fall back to zero-config defaults.
    assert!(
        out.stderr.contains("config not found"),
        "stderr: {}",
        out.stderr
    );
}

#[test]
fn test_build_agent_provider_rejects_models_json_remote_http_by_default() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"baseUrl":"http://example.com/v1","apiKey":"sk-fw","api":"openai-completions","models":[{"id":"m"}]}}}"#,
    )
    .unwrap();
    let config = config_from_str(r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#);

    let result = build_agent_provider(&config, tmp.path(), &reqwest::Client::new());

    let err = result.unwrap_err();
    assert!(
        err.contains("models.json provider configuration error"),
        "{err}"
    );
    assert!(
        err.contains("http is allowed only for loopback hosts"),
        "{err}"
    );
}

#[test]
fn test_build_agent_provider_allows_models_json_remote_http_when_explicit() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"fireworks":{"baseUrl":"http://example.com/v1","apiKey":"sk-fw","api":"openai-completions","allowRemoteHttp":true,"models":[{"id":"m"}]}}}"#,
    )
    .unwrap();
    let config = config_from_str(r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#);

    let result = build_agent_provider(&config, tmp.path(), &reqwest::Client::new());

    assert!(result.is_ok(), "{result:?}");
}

/// Downcast the built provider to a `ProviderRouter` and return its provider names.
fn router_provider_names(
    provider: &std::sync::Arc<dyn crate::domain::provider::LlmProvider>,
) -> Vec<String> {
    let router = provider
        .as_any()
        .downcast_ref::<crate::infrastructure::providers::router::ProviderRouter>()
        .expect("build_agent_provider should return a ProviderRouter");
    router
        .provider_names()
        .into_iter()
        .map(|s| s.to_string())
        .collect()
}

#[test]
fn test_build_agent_provider_registry_anthropic_api_key_provider() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"anthropic-api":{"api":"anthropic-messages","baseUrl":"https://api.anthropic.com","auth":{"mode":"apiKey","apiKey":"sk-ant-direct"},"models":[{"id":"claude-opus-4-8"}]}}}"#,
    )
    .unwrap();
    let config = config_from_str(r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#);

    let provider = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap();
    let names = router_provider_names(&provider);
    assert!(
        names.iter().any(|n| n == "anthropic-api"),
        "expected anthropic-api provider, got: {names:?}"
    );
}

#[test]
fn test_build_agent_provider_registry_oauth_provider() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "anthropic".to_string(),
            token: "sk-ant-oat01-valid".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(i64::MAX),
            refresh_token: Some("rt".to_string()),
            account_id: None,
        })
        .unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"anthropic-oauth":{"api":"anthropic-messages","auth":{"mode":"oauth","oauthProvider":"anthropic"},"models":[{"id":"claude-opus-4-8"}]}}}"#,
    )
    .unwrap();
    let config = config_from_str(r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#);

    let provider = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap();
    let names = router_provider_names(&provider);
    assert!(
        names.iter().any(|n| n == "anthropic-oauth"),
        "expected anthropic-oauth provider, got: {names:?}"
    );
}

#[test]
fn test_build_agent_provider_registry_oauth_unknown_provider_rejected() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"cohere-oauth":{"api":"openai-completions","auth":{"mode":"oauth","oauthProvider":"cohere"},"models":[{"id":"m"}]}}}"#,
    )
    .unwrap();
    let config = config_from_str(r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#);

    let result = build_agent_provider(&config, tmp.path(), &reqwest::Client::new());
    let err = result.unwrap_err();
    assert!(
        err.contains("not a kernel OAuth provider"),
        "expected kernel-OAuth rejection, got: {err}"
    );
}

#[test]
fn test_build_agent_provider_registry_oauth_rejects_non_canonical_base_url() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "anthropic".to_string(),
            token: "sk-ant-oat01-valid".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(i64::MAX),
            refresh_token: Some("rt".to_string()),
            account_id: None,
        })
        .unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"evil-oauth":{"api":"anthropic-messages","baseUrl":"https://attacker.example","auth":{"mode":"oauth","oauthProvider":"anthropic"},"models":[{"id":"claude-opus-4-8"}]}}}"#,
    )
    .unwrap();
    let config = config_from_str(r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#);

    let result = build_agent_provider(&config, tmp.path(), &reqwest::Client::new());
    let err = result.unwrap_err();
    assert!(
        err.contains("not the canonical OAuth host"),
        "expected canonical-host rejection, got: {err}"
    );
}

#[test]
fn test_build_agent_provider_registry_openai_oauth_uses_default_base_url() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "openai".to_string(),
            token: "sk-oauth-token".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(i64::MAX),
            refresh_token: Some("rt".to_string()),
            account_id: None,
        })
        .unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"openai-oauth-custom":{"api":"openai-completions","auth":{"mode":"oauth","oauthProvider":"openai"},"models":[{"id":"gpt-5.5"}]}}}"#,
    )
    .unwrap();
    let config = config_from_str(r#"{"providers":{"anthropic":{"api_key":"sk-ant"}}}"#);

    let provider = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap();
    let names = router_provider_names(&provider);
    assert!(
        names.iter().any(|n| n == "openai-oauth-custom"),
        "got: {names:?}"
    );
}

// ===================================================================
// Issue #811: OAuth refresh must be lazy — never on the build (pre-announce)
// critical path. build_agent_provider must construct providers from the
// stored (possibly stale/expired) token WITHOUT any network refresh; the
// existing RefreshableProvider refreshes on-demand on a 401 at first use,
// AFTER the socket is announced.
// ===================================================================

/// Store an expired OAuth credential whose refresh token is bogus. If
/// `build_agent_provider` eagerly refreshes (the #811 regression) the refresh
/// fails (invalid_grant / network) and the built-in `<vendor>-oauth` provider is
/// dropped. With lazy construction the provider is present (stale token), and
/// refresh is deferred to first request.
fn store_expired_oauth(tmp: &std::path::Path, provider: &str, token: &str) {
    let store = CredentialStore::new(tmp);
    store
        .store(Credential {
            provider: provider.to_string(),
            token: token.to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(0), // epoch — always expired
            refresh_token: Some("bogus-refresh-token".to_string()),
            account_id: None,
        })
        .unwrap();
}

#[test]
fn test_build_agent_provider_expired_anthropic_oauth_constructed_without_refresh() {
    let tmp = tempfile::TempDir::new().unwrap();
    store_expired_oauth(tmp.path(), "anthropic", "sk-ant-oat01-stale");
    // Empty registry so only the built-in anthropic-oauth path is exercised.
    std::fs::write(tmp.path().join("models.json"), r#"{"providers":{}}"#).unwrap();
    let config = config_from_str(r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#);

    let start = std::time::Instant::now();
    let provider = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap();
    let elapsed = start.elapsed();

    let names = router_provider_names(&provider);
    assert!(
        names.iter().any(|n| n == "anthropic-oauth"),
        "expired anthropic OAuth credential must still produce an anthropic-oauth \
         provider built from the stale token (no eager refresh); got: {names:?}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "build_agent_provider must not perform a blocking OAuth network refresh on \
         the startup path (took {elapsed:?})"
    );
}

#[test]
fn test_build_agent_provider_expired_openai_oauth_constructed_without_refresh() {
    let tmp = tempfile::TempDir::new().unwrap();
    // Use a JWT-shaped OAuth token so the codex routing path is exercised too.
    store_expired_oauth(tmp.path(), "openai", &openai_oauth_jwt("acct-stale"));
    std::fs::write(tmp.path().join("models.json"), r#"{"providers":{}}"#).unwrap();
    let config = config_from_str(r#"{"providers":{"anthropic":{"api_key":"sk-ant"}}}"#);

    let start = std::time::Instant::now();
    let provider = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap();
    let elapsed = start.elapsed();

    let names = router_provider_names(&provider);
    assert!(
        names.iter().any(|n| n == "openai-oauth"),
        "expired openai OAuth credential must still produce an openai-oauth provider \
         built from the stale token (no eager refresh); got: {names:?}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "build_agent_provider must not perform a blocking OAuth network refresh on \
         the startup path (took {elapsed:?})"
    );
}

/// Issue #811 de-duplication: the double-refresh in the trace came from the SAME
/// OAuth credential backing both the built-in `anthropic-oauth` provider and a
/// `models.json` provider also named `anthropic-oauth`. The registry loop must
/// skip a provider whose name is already present, so the vendor is constructed
/// exactly once — never twice. (With lazy construction the refresh is gone
/// entirely; this guards the structural de-dup that previously multiplied the
/// stall.)
#[test]
fn test_build_agent_provider_oauth_provider_constructed_exactly_once() {
    let tmp = tempfile::TempDir::new().unwrap();
    store_expired_oauth(tmp.path(), "anthropic", "sk-ant-oat01-stale");
    // models.json redeclares anthropic-oauth referencing the same kernel
    // credential — without de-dup this would build the provider a second time.
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"anthropic-oauth":{"api":"anthropic-messages","auth":{"mode":"oauth","oauthProvider":"anthropic"},"models":[{"id":"claude-opus-4-8"}]}}}"#,
    )
    .unwrap();
    let config = config_from_str(r#"{"providers":{}}"#);

    let provider = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap();

    let names = router_provider_names(&provider);
    let count = names.iter().filter(|n| *n == "anthropic-oauth").count();
    assert_eq!(
        count, 1,
        "anthropic-oauth must be constructed exactly once (no duplicate from the \
         models.json registry loop); got names: {names:?}"
    );
}

#[test]
fn test_build_agent_provider_oauth_and_api_key_coexist_for_same_vendor() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "anthropic".to_string(),
            token: "sk-ant-oat01-valid".to_string(),
            method: AuthMethod::OAuth,
            expires_at: Some(i64::MAX),
            refresh_token: Some("rt".to_string()),
            account_id: None,
        })
        .unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{
            "anthropic-oauth":{"api":"anthropic-messages","auth":{"mode":"oauth","oauthProvider":"anthropic"},"models":[{"id":"claude-opus-4-8"}]},
            "anthropic-api":{"api":"anthropic-messages","baseUrl":"https://api.anthropic.com","auth":{"mode":"apiKey","apiKey":"sk-ant-direct"},"models":[{"id":"claude-opus-4-8"}]}
        }}"#,
    )
    .unwrap();
    let config = config_from_str(r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#);

    let provider = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap();
    let names = router_provider_names(&provider);
    assert!(
        names.iter().any(|n| n == "anthropic-oauth"),
        "got: {names:?}"
    );
    assert!(names.iter().any(|n| n == "anthropic-api"), "got: {names:?}");
}
