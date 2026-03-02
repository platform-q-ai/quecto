use super::*;
use crate::infrastructure::config::Config;

#[test]
fn test_gateway_creation() {
    let config: Config = serde_json::from_str("{}").unwrap();
    let gw = Gateway::new(config, PathBuf::from("/tmp/quecto-test"));
    assert_eq!(gw.base_dir, PathBuf::from("/tmp/quecto-test"));
}

#[test]
fn test_resolve_workspace_default() {
    let config: Config = serde_json::from_str(
        r#"{
        "agents": { "defaults": { "workspace": "/opt/workspace" } }
    }"#,
    )
    .unwrap();
    let gw = Gateway::new(config, PathBuf::from("/tmp/test"));
    let ws = gw.resolve_workspace();
    assert_eq!(ws, PathBuf::from("/opt/workspace"));
}

#[tokio::test]
async fn test_gateway_no_providers_error() {
    let config: Config = serde_json::from_str("{}").unwrap();
    let gw = Gateway::new(config, PathBuf::from("/tmp/quecto-test"));
    let result = gw.run().await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("no LLM providers"),
        "expected NoProviders, got: {}",
        err
    );
}

#[test]
fn test_resolve_api_key_from_credential_store() {
    use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "openai".to_string(),
            token: "sk-from-store".to_string(),
            method: AuthMethod::Token,
            expires_at: None,
            refresh_token: None,
            account_id: None,
        })
        .unwrap();

    let creds = store.load_snapshot().unwrap();
    let config: Config = serde_json::from_str("{}").unwrap();
    let resolved = resolve_api_key(&config.providers.openai.api_key, &creds, "openai");
    assert_eq!(resolved, "sk-from-store");
}

#[test]
fn test_resolve_api_key_prefers_store_over_config() {
    use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "openai".to_string(),
            token: "sk-from-store".to_string(),
            method: AuthMethod::Token,
            expires_at: None,
            refresh_token: None,
            account_id: None,
        })
        .unwrap();

    let creds = store.load_snapshot().unwrap();
    let config: Config =
        serde_json::from_str(r#"{"providers": {"openai": {"api_key": "sk-from-config"}}}"#)
            .unwrap();
    let resolved = resolve_api_key(&config.providers.openai.api_key, &creds, "openai");
    assert_eq!(resolved, "sk-from-store");
}

#[test]
fn test_resolve_api_key_falls_back_to_config() {
    use crate::infrastructure::auth::credential_store::CredentialStore;
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    // No credential stored

    let creds = store.load_snapshot().unwrap();
    let config: Config =
        serde_json::from_str(r#"{"providers": {"openai": {"api_key": "sk-from-config"}}}"#)
            .unwrap();
    let resolved = resolve_api_key(&config.providers.openai.api_key, &creds, "openai");
    assert_eq!(resolved, "sk-from-config");
}

#[test]
fn test_resolve_api_key_ignores_expired_credential() {
    use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "openai".to_string(),
            token: "sk-expired".to_string(),
            method: AuthMethod::Token,
            expires_at: Some(0), // always expired
            refresh_token: None,
            account_id: None,
        })
        .unwrap();

    let creds = store.load_snapshot().unwrap();
    let config: Config =
        serde_json::from_str(r#"{"providers": {"openai": {"api_key": "sk-from-config"}}}"#)
            .unwrap();
    let resolved = resolve_api_key(&config.providers.openai.api_key, &creds, "openai");
    assert_eq!(resolved, "sk-from-config");
}

#[test]
fn test_check_provider_readiness_reports_expired() {
    use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "openai".to_string(),
            token: "sk-expired".to_string(),
            method: AuthMethod::Token,
            expires_at: Some(0),
            refresh_token: None,
            account_id: None,
        })
        .unwrap();

    let creds = store.load_snapshot().unwrap();
    let needs_reauth = check_provider_readiness(&creds);
    assert!(needs_reauth.contains(&"openai".to_string()));
}

// --- Bot command tests ---

#[test]
fn test_handle_bot_command_start() {
    let config: Config = serde_json::from_str("{}").unwrap();
    let result = handle_bot_command("/start", &config);
    assert!(result.is_some(), "/start should be handled");
    let text = result.unwrap();
    assert!(
        text.contains("quecto"),
        "start response should mention quecto"
    );
    assert!(
        text.contains("Welcome"),
        "start response should be welcoming"
    );
}

#[test]
fn test_handle_bot_command_help() {
    let config: Config = serde_json::from_str("{}").unwrap();
    let result = handle_bot_command("/help", &config);
    assert!(result.is_some(), "/help should be handled");
    let text = result.unwrap();
    assert!(text.contains("/start"), "help should list /start");
    assert!(text.contains("/help"), "help should list /help");
    assert!(text.contains("/status"), "help should list /status");
}

#[test]
fn test_handle_bot_command_status() {
    let config: Config =
        serde_json::from_str(r#"{"agents": {"defaults": {"model": "gpt-5.2"}}}"#).unwrap();
    let result = handle_bot_command("/status", &config);
    assert!(result.is_some(), "/status should be handled");
    let text = result.unwrap();
    assert!(text.contains("Model:"), "status should show model");
    assert!(text.contains("gpt-5.2"), "status should show model name");
}

#[test]
fn test_handle_bot_command_unknown_returns_none() {
    let config: Config = serde_json::from_str("{}").unwrap();
    let result = handle_bot_command("/unknown", &config);
    assert!(result.is_none(), "/unknown should not be handled");
}

#[test]
fn test_handle_bot_command_regular_text_returns_none() {
    let config: Config = serde_json::from_str("{}").unwrap();
    let result = handle_bot_command("Hello, how are you?", &config);
    assert!(result.is_none(), "regular text should not be handled");
}

#[test]
fn test_handle_bot_command_start_with_args() {
    let config: Config = serde_json::from_str("{}").unwrap();
    // /start with args (deep link) should still be handled
    let result = handle_bot_command("/start ref123", &config);
    assert!(result.is_some(), "/start with args should be handled");
}

#[test]
fn test_check_provider_readiness_active_is_empty() {
    use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};
    let tmp = tempfile::TempDir::new().unwrap();
    let store = CredentialStore::new(tmp.path());
    store
        .store(Credential {
            provider: "openai".to_string(),
            token: "sk-active".to_string(),
            method: AuthMethod::Token,
            expires_at: None,
            refresh_token: None,
            account_id: None,
        })
        .unwrap();

    let creds = store.load_snapshot().unwrap();
    let needs_reauth = check_provider_readiness(&creds);
    assert!(needs_reauth.is_empty());
}

#[tokio::test]
async fn test_run_health_server_starts_and_responds() {
    use crate::infrastructure::config::HealthConfig;

    // Bind to port 0 to get a random available port
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind to random port");
    let port = listener.local_addr().unwrap().port();
    drop(listener); // Release port so health server can bind to it

    let config = HealthConfig {
        enabled: true,
        port,
    };

    // Spawn health server in background
    let handle = tokio::spawn(Gateway::run_health_server(config));

    // Wait briefly for the server to start
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Make a request
    let client = reqwest::Client::new();
    let resp = client
        .get(format!("http://127.0.0.1:{}/health", port))
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["status"], "ok");

    // Ready should return true (gateway sets readiness to true)
    let resp = client
        .get(format!("http://127.0.0.1:{}/ready", port))
        .send()
        .await
        .expect("request should succeed");
    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["ready"], true);

    handle.abort();
}

#[tokio::test]
async fn test_run_health_server_disabled_suspends() {
    use crate::infrastructure::config::HealthConfig;

    let config = HealthConfig {
        enabled: false,
        port: 0,
    };

    // Should not return — just suspend forever. We verify by racing with a timeout.
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        Gateway::run_health_server(config),
    )
    .await;

    assert!(
        result.is_err(),
        "disabled health server should suspend (timeout expected)"
    );
}

// --- GatewayError Display tests ---

#[test]
fn test_gateway_error_display_config() {
    let err = GatewayError::Config("bad value".to_string());
    assert_eq!(err.to_string(), "config error: bad value");
}

#[test]
fn test_gateway_error_display_no_providers() {
    let err = GatewayError::NoProviders;
    assert_eq!(err.to_string(), "no LLM providers configured");
}

#[test]
fn test_gateway_error_display_runtime() {
    let err = GatewayError::Runtime("connection lost".to_string());
    assert_eq!(err.to_string(), "runtime error: connection lost");
}

#[test]
fn test_gateway_error_implements_std_error() {
    let err = GatewayError::Config("test".to_string());
    // Verify it implements std::error::Error by calling source()
    let as_error: &dyn std::error::Error = &err;
    assert!(as_error.source().is_none());
}

#[test]
fn test_gateway_error_debug_format() {
    let err = GatewayError::NoProviders;
    let debug = format!("{:?}", err);
    assert!(debug.contains("NoProviders"));
}

// --- brave_api_key tests ---

#[test]
fn test_brave_api_key_enabled_with_key() {
    let config: Config = serde_json::from_str(
        r#"{"tools": {"web": {"brave": {"enabled": true, "api_key": "brave-key-123"}}}}"#,
    )
    .unwrap();
    let gw = Gateway::new(config, PathBuf::from("/tmp/test"));
    assert_eq!(gw.brave_api_key(), Some("brave-key-123".to_string()));
}

#[test]
fn test_brave_api_key_disabled() {
    let config: Config = serde_json::from_str(
        r#"{"tools": {"web": {"brave": {"enabled": false, "api_key": "brave-key-123"}}}}"#,
    )
    .unwrap();
    let gw = Gateway::new(config, PathBuf::from("/tmp/test"));
    assert_eq!(gw.brave_api_key(), None);
}

#[test]
fn test_brave_api_key_enabled_but_empty() {
    let config: Config =
        serde_json::from_str(r#"{"tools": {"web": {"brave": {"enabled": true, "api_key": ""}}}}"#)
            .unwrap();
    let gw = Gateway::new(config, PathBuf::from("/tmp/test"));
    assert_eq!(gw.brave_api_key(), None);
}

#[test]
fn test_brave_api_key_default_config() {
    let config: Config = serde_json::from_str("{}").unwrap();
    let gw = Gateway::new(config, PathBuf::from("/tmp/test"));
    // Default BraveConfig has enabled=false, api_key=""
    assert_eq!(gw.brave_api_key(), None);
}

// --- build_whisper_client tests ---
// These tests pass `allow_insecure` as a parameter instead of manipulating env vars.

#[test]
fn test_build_whisper_client_no_key() {
    use crate::infrastructure::config::VoiceConfig;
    let voice = VoiceConfig::default();
    assert!(voice.groq.api_key.is_empty());
    let result = Gateway::build_whisper_client(&voice, false);
    assert!(result.is_none(), "should return None when api_key is empty");
}

#[test]
fn test_build_whisper_client_with_key_no_base() {
    use crate::infrastructure::config::{GroqVoiceConfig, VoiceConfig};
    let voice = VoiceConfig {
        groq: GroqVoiceConfig {
            api_key: "gsk-test-key".to_string(),
            api_base: String::new(),
        },
    };
    let result = Gateway::build_whisper_client(&voice, false);
    assert!(
        result.is_some(),
        "should return Some when key present and base empty"
    );
}

#[test]
fn test_build_whisper_client_with_key_and_valid_https_base() {
    use crate::infrastructure::config::{GroqVoiceConfig, VoiceConfig};
    let voice = VoiceConfig {
        groq: GroqVoiceConfig {
            api_key: "gsk-test-key".to_string(),
            api_base: "https://api.groq.com/openai/v1".to_string(),
        },
    };
    let result = Gateway::build_whisper_client(&voice, false);
    assert!(
        result.is_some(),
        "should return Some for valid https URL to api.groq.com"
    );
}

#[test]
fn test_build_whisper_client_http_without_insecure_flag() {
    use crate::infrastructure::config::{GroqVoiceConfig, VoiceConfig};
    let voice = VoiceConfig {
        groq: GroqVoiceConfig {
            api_key: "gsk-test-key".to_string(),
            api_base: "http://localhost:8080/v1".to_string(),
        },
    };
    let result = Gateway::build_whisper_client(&voice, false);
    assert!(
        result.is_none(),
        "should reject http URL without insecure override"
    );
}

#[test]
fn test_build_whisper_client_http_with_insecure_flag() {
    use crate::infrastructure::config::{GroqVoiceConfig, VoiceConfig};
    let voice = VoiceConfig {
        groq: GroqVoiceConfig {
            api_key: "gsk-test-key".to_string(),
            api_base: "http://localhost:8080/v1".to_string(),
        },
    };
    let result = Gateway::build_whisper_client(&voice, true);
    assert!(
        result.is_some(),
        "should allow http URL when insecure flag is set"
    );
}

#[test]
fn test_build_whisper_client_invalid_url() {
    use crate::infrastructure::config::{GroqVoiceConfig, VoiceConfig};
    let voice = VoiceConfig {
        groq: GroqVoiceConfig {
            api_key: "gsk-test-key".to_string(),
            api_base: "not a valid url ://".to_string(),
        },
    };
    let result = Gateway::build_whisper_client(&voice, false);
    assert!(result.is_none(), "should return None for invalid URL");
}

#[test]
fn test_build_whisper_client_url_with_credentials() {
    use crate::infrastructure::config::{GroqVoiceConfig, VoiceConfig};
    let voice = VoiceConfig {
        groq: GroqVoiceConfig {
            api_key: "gsk-test-key".to_string(),
            api_base: "https://user:pass@api.groq.com/v1".to_string(),
        },
    };
    let result = Gateway::build_whisper_client(&voice, false);
    assert!(
        result.is_none(),
        "should reject URL with embedded credentials"
    );
}

#[test]
fn test_build_whisper_client_url_with_query() {
    use crate::infrastructure::config::{GroqVoiceConfig, VoiceConfig};
    let voice = VoiceConfig {
        groq: GroqVoiceConfig {
            api_key: "gsk-test-key".to_string(),
            api_base: "https://api.groq.com/v1?token=abc".to_string(),
        },
    };
    let result = Gateway::build_whisper_client(&voice, false);
    assert!(result.is_none(), "should reject URL with query parameters");
}

#[test]
fn test_build_whisper_client_url_with_fragment() {
    use crate::infrastructure::config::{GroqVoiceConfig, VoiceConfig};
    let voice = VoiceConfig {
        groq: GroqVoiceConfig {
            api_key: "gsk-test-key".to_string(),
            api_base: "https://api.groq.com/v1#section".to_string(),
        },
    };
    let result = Gateway::build_whisper_client(&voice, false);
    assert!(result.is_none(), "should reject URL with fragment");
}

#[test]
fn test_build_whisper_client_https_non_groq_host_rejected() {
    use crate::infrastructure::config::{GroqVoiceConfig, VoiceConfig};
    // Without insecure flag, non-groq hosts should be rejected
    let voice = VoiceConfig {
        groq: GroqVoiceConfig {
            api_key: "gsk-test-key".to_string(),
            api_base: "https://evil.example.com/v1".to_string(),
        },
    };
    let result = Gateway::build_whisper_client(&voice, false);
    assert!(
        result.is_none(),
        "should reject non-groq host without insecure flag"
    );
}

#[test]
fn test_build_whisper_client_https_non_groq_host_with_insecure_flag() {
    use crate::infrastructure::config::{GroqVoiceConfig, VoiceConfig};
    // With insecure flag, non-groq https hosts should be allowed
    let voice = VoiceConfig {
        groq: GroqVoiceConfig {
            api_key: "gsk-test-key".to_string(),
            api_base: "https://my-proxy.example.com/v1".to_string(),
        },
    };
    let result = Gateway::build_whisper_client(&voice, true);
    assert!(
        result.is_some(),
        "should allow non-groq https host with insecure flag"
    );
}

// --- build_fallback_provider tests ---

#[test]
fn test_build_fallback_provider_no_keys() {
    let config: Config = serde_json::from_str("{}").unwrap();
    let tmp = tempfile::TempDir::new().unwrap();
    let gw = Gateway::new(config, tmp.path().to_path_buf());
    let creds = std::collections::HashMap::new();
    let result = gw.build_fallback_provider(&creds, &reqwest::Client::new());
    assert!(result.is_err(), "should fail with no providers configured");
    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("no LLM providers"),
        "expected NoProviders error, got: {}",
        err
    );
}

#[test]
fn test_build_fallback_provider_openai_only() {
    let config: Config =
        serde_json::from_str(r#"{"providers": {"openai": {"api_key": "sk-test-openai"}}}"#)
            .unwrap();
    let tmp = tempfile::TempDir::new().unwrap();
    let gw = Gateway::new(config, tmp.path().to_path_buf());
    let creds = std::collections::HashMap::new();
    let result = gw.build_fallback_provider(&creds, &reqwest::Client::new());
    assert!(result.is_ok(), "should succeed with openai key");
}

#[test]
fn test_build_fallback_provider_anthropic_only() {
    let config: Config =
        serde_json::from_str(r#"{"providers": {"anthropic": {"api_key": "sk-ant-test"}}}"#)
            .unwrap();
    let tmp = tempfile::TempDir::new().unwrap();
    let gw = Gateway::new(config, tmp.path().to_path_buf());
    let creds = std::collections::HashMap::new();
    let result = gw.build_fallback_provider(&creds, &reqwest::Client::new());
    assert!(result.is_ok(), "should succeed with anthropic key");
}

#[test]
fn test_build_fallback_provider_both_providers() {
    let config: Config = serde_json::from_str(
        r#"{"providers": {"openai": {"api_key": "sk-openai"}, "anthropic": {"api_key": "sk-ant"}}}"#,
    )
    .unwrap();
    let tmp = tempfile::TempDir::new().unwrap();
    let gw = Gateway::new(config, tmp.path().to_path_buf());
    let creds = std::collections::HashMap::new();
    let result = gw.build_fallback_provider(&creds, &reqwest::Client::new());
    assert!(result.is_ok(), "should succeed with both providers");
}
