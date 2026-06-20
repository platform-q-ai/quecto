use super::*;

#[test]
fn test_openai_config() {
    let config = OAuthConfig::for_provider("openai").unwrap();
    assert!(config.authorization_url.contains("openai.com"));
    assert!(config.device_code_url.contains("openai.com"));
    assert_eq!(config.client_id, "app_EMoamEEZ73f0CkXaXp7hrann");
}

#[test]
fn test_anthropic_config() {
    let config = OAuthConfig::for_provider("anthropic").unwrap();
    assert!(config.authorization_url.contains("claude.ai"));
    assert!(config.token_url.contains("console.anthropic.com"));
    assert_eq!(config.client_id, "9d1c250a-e61b-44d9-88ed-5944d1962f5e");
    assert!(config.scopes.contains("user:inference"));
}

#[test]
fn test_unknown_provider_returns_none() {
    assert!(OAuthConfig::for_provider("groq").is_none());
}

#[test]
fn test_custom_base_url() {
    let config = OAuthConfig::with_base_url("http://localhost:9999");
    assert_eq!(config.authorization_url, "http://localhost:9999/authorize");
    assert_eq!(config.device_code_url, "http://localhost:9999/device/code");
    assert_eq!(config.token_url, "http://localhost:9999/oauth/token");
}

#[test]
fn test_generate_pkce() {
    let pkce = generate_pkce();
    assert!(!pkce.verifier.is_empty());
    assert!(!pkce.challenge.is_empty());
    assert_ne!(pkce.verifier, pkce.challenge);
}

#[test]
fn test_pkce_deterministic_challenge() {
    // Verify that the same verifier produces the same challenge
    use base64::Engine;
    use sha2::Digest;

    let pkce = generate_pkce();
    let mut hasher = sha2::Sha256::new();
    hasher.update(pkce.verifier.as_bytes());
    let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize());
    assert_eq!(pkce.challenge, expected);
}

#[test]
fn test_build_anthropic_auth_url() {
    let config = OAuthConfig::for_provider("anthropic").unwrap();
    let pkce = PkceCodes {
        verifier: "test-verifier".into(),
        challenge: "test-challenge".into(),
    };
    let url = build_anthropic_auth_url(&config, &pkce, "test-state");
    assert!(url.starts_with("https://claude.ai/oauth/authorize?"));
    assert!(url.contains("client_id="));
    assert!(url.contains("code_challenge=test-challenge"));
    assert!(url.contains("response_type=code"));
    assert!(url.contains("redirect_uri="));
    assert!(url.contains("state=test-state"));
    // Verify PKCE verifier is NOT in the URL
    assert!(!url.contains("test-verifier"));
}

#[test]
fn test_is_anthropic_oauth_token() {
    assert!(is_anthropic_oauth_token("sk-ant-oat01-abc123"));
    assert!(!is_anthropic_oauth_token("sk-ant-api03-abc123"));
    assert!(!is_anthropic_oauth_token("sk-proj-abc123"));
}

#[tokio::test]
async fn test_device_code_success() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response = serde_json::json!({
        "device_code": "DEVCODE123",
        "user_code": "ABCD-1234",
        "verification_uri": "https://example.com/device"
    });

    Mock::given(method("POST"))
        .and(path("/device/code"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let config = OAuthConfig::with_base_url(&server.uri());
    let result = request_device_code(&config).await.unwrap();
    assert_eq!(result.device_code, "DEVCODE123");
    assert_eq!(result.user_code, "ABCD-1234");
    assert_eq!(result.verification_uri, "https://example.com/device");
}

#[tokio::test]
async fn test_device_code_server_error() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/device/code"))
        .respond_with(ResponseTemplate::new(500).set_body_string("fail"))
        .mount(&server)
        .await;

    let config = OAuthConfig::with_base_url(&server.uri());
    let result = request_device_code(&config).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_exchange_anthropic_code_success() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response = serde_json::json!({
        "access_token": "sk-ant-oat01-test-access",
        "refresh_token": "sk-ant-ort01-test-refresh",
        "expires_in": 28800
    });

    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let config = OAuthConfig::with_base_url(&server.uri());
    let result = exchange_anthropic_code(&config, "code123#state456", "verifier")
        .await
        .unwrap();
    assert_eq!(result.access_token, "sk-ant-oat01-test-access");
    assert_eq!(
        result.refresh_token,
        Some("sk-ant-ort01-test-refresh".to_string())
    );
    assert_eq!(result.expires_in, 28800);
}

#[tokio::test]
async fn test_refresh_anthropic_token_success() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response = serde_json::json!({
        "access_token": "sk-ant-oat01-new-access",
        "refresh_token": "sk-ant-ort01-new-refresh",
        "expires_in": 28800
    });

    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let config = OAuthConfig::with_base_url(&server.uri());
    let result = refresh_anthropic_token(&config, "sk-ant-ort01-old-refresh")
        .await
        .unwrap();
    assert_eq!(result.access_token, "sk-ant-oat01-new-access");
    assert_eq!(
        result.refresh_token,
        Some("sk-ant-ort01-new-refresh".to_string())
    );
}

#[tokio::test]
async fn test_refresh_anthropic_token_failure() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(400).set_body_string("invalid_grant"))
        .mount(&server)
        .await;

    let config = OAuthConfig::with_base_url(&server.uri());
    let result = refresh_anthropic_token(&config, "bad-refresh").await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("400"));
}

#[test]
fn test_oauth_token_response_deserializes_with_refresh_token() {
    let json = r#"{"access_token":"at-123","refresh_token":"rt-456","expires_in":3600}"#;
    let resp: OAuthTokenResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.access_token, "at-123");
    assert_eq!(resp.refresh_token, Some("rt-456".to_string()));
    assert_eq!(resp.expires_in, 3600);
}

#[test]
fn test_oauth_token_response_deserializes_without_refresh_token() {
    let json = r#"{"access_token":"at-789","expires_in":7200}"#;
    let resp: OAuthTokenResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.access_token, "at-789");
    assert_eq!(resp.refresh_token, None);
    assert_eq!(resp.expires_in, 7200);
}

#[test]
fn test_oauth_token_response_deserializes_with_null_refresh_token() {
    let json = r#"{"access_token":"at-abc","refresh_token":null,"expires_in":1800}"#;
    let resp: OAuthTokenResponse = serde_json::from_str(json).unwrap();
    assert_eq!(resp.access_token, "at-abc");
    assert_eq!(resp.refresh_token, None);
    assert_eq!(resp.expires_in, 1800);
}

#[tokio::test]
async fn test_refresh_anthropic_token_without_refresh_token_in_response() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response = serde_json::json!({
        "access_token": "sk-ant-oat01-new-access",
        "expires_in": 28800
        // No refresh_token field — server says "keep using the old one"
    });

    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let config = OAuthConfig::with_base_url(&server.uri());
    let result = refresh_anthropic_token(&config, "sk-ant-ort01-old-refresh")
        .await
        .unwrap();
    assert_eq!(result.access_token, "sk-ant-oat01-new-access");
    assert_eq!(result.refresh_token, None);
    assert_eq!(result.expires_in, 28800);
}

// ===================================================================
// build_openai_auth_url / generate_state (pure)
// ===================================================================

#[test]
fn test_build_openai_auth_url() {
    let config = OAuthConfig::for_provider("openai").unwrap();
    let pkce = PkceCodes {
        verifier: "secret-verifier".into(),
        challenge: "chal".into(),
    };
    let url = build_openai_auth_url(&config, &pkce, "st8");
    assert!(url.starts_with("https://auth.openai.com/oauth/authorize?"));
    assert!(url.contains("response_type=code"));
    assert!(url.contains("code_challenge=chal"));
    assert!(url.contains("code_challenge_method=S256"));
    assert!(url.contains("state=st8"));
    assert!(url.contains("id_token_add_organizations=true"));
    assert!(url.contains("codex_cli_simplified_flow=true"));
    assert!(url.contains("originator=codex_cli_rs"));
    // PKCE verifier must never leak into the URL
    assert!(!url.contains("secret-verifier"));
}

#[test]
fn test_generate_state_is_hex_and_random() {
    let s = generate_state();
    assert_eq!(s.len(), 32);
    assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    assert_ne!(generate_state(), generate_state());
}

// ===================================================================
// extract_openai_account_id (pure JWT parsing)
// ===================================================================

fn jwt_with_payload(payload: &serde_json::Value) -> String {
    use base64::Engine;
    let enc = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(payload).unwrap());
    format!("header.{}.signature", enc)
}

#[test]
fn test_extract_openai_account_id_not_three_parts() {
    assert!(extract_openai_account_id("not-a-jwt").is_none());
    assert!(extract_openai_account_id("only.two").is_none());
    assert!(extract_openai_account_id("a.b.c.d").is_none());
}

#[test]
fn test_extract_openai_account_id_bad_base64() {
    assert!(extract_openai_account_id("aaa.!!!@@@.bbb").is_none());
}

#[test]
fn test_extract_openai_account_id_payload_not_json() {
    use base64::Engine;
    let enc = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"plain text not json");
    assert!(extract_openai_account_id(&format!("h.{}.s", enc)).is_none());
}

#[test]
fn test_extract_openai_account_id_valid() {
    let payload = serde_json::json!({
        "https://api.openai.com/auth": { "chatgpt_account_id": "acct-xyz" }
    });
    let token = jwt_with_payload(&payload);
    assert_eq!(
        extract_openai_account_id(&token),
        Some("acct-xyz".to_string())
    );
}

#[test]
fn test_extract_openai_account_id_empty_value() {
    let payload = serde_json::json!({
        "https://api.openai.com/auth": { "chatgpt_account_id": "" }
    });
    assert!(extract_openai_account_id(&jwt_with_payload(&payload)).is_none());
}

#[test]
fn test_extract_openai_account_id_missing_claim() {
    let payload = serde_json::json!({ "sub": "user-123" });
    assert!(extract_openai_account_id(&jwt_with_payload(&payload)).is_none());
}

// ===================================================================
// DeviceCodeResponse Debug redaction
// ===================================================================

#[test]
fn test_device_code_response_debug_redacts_secret() {
    let resp = DeviceCodeResponse {
        device_code: "SECRET-DEVICE-CODE".to_string(),
        user_code: "USER-1234".to_string(),
        verification_uri: "https://example.com/verify".to_string(),
    };
    let dbg = format!("{:?}", resp);
    assert!(dbg.contains("[REDACTED]"));
    assert!(!dbg.contains("SECRET-DEVICE-CODE"));
    assert!(dbg.contains("USER-1234"));
    assert!(dbg.contains("example.com/verify"));
}

// ===================================================================
// OpenAI token exchange / refresh (wiremock-backed Ok + Err arms)
// ===================================================================

#[tokio::test]
async fn test_exchange_openai_code_success() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response = serde_json::json!({
        "access_token": "oai-access",
        "refresh_token": "oai-refresh",
        "expires_in": 3600
    });
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let config = OAuthConfig::with_base_url(&server.uri());
    let result = exchange_openai_code(&config, "code", "verifier")
        .await
        .unwrap();
    assert_eq!(result.access_token, "oai-access");
    assert_eq!(result.refresh_token, Some("oai-refresh".to_string()));
    assert_eq!(result.expires_in, 3600);
}

#[tokio::test]
async fn test_exchange_openai_code_error_status() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(400).set_body_string("invalid_grant"))
        .mount(&server)
        .await;

    let config = OAuthConfig::with_base_url(&server.uri());
    let err = exchange_openai_code(&config, "bad", "verifier")
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("400"));
    assert!(msg.contains("OpenAI token exchange failed"));
}

#[tokio::test]
async fn test_exchange_openai_code_parse_error() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not valid json"))
        .mount(&server)
        .await;

    let config = OAuthConfig::with_base_url(&server.uri());
    let err = exchange_openai_code(&config, "code", "verifier")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("parse"));
}

#[tokio::test]
async fn test_refresh_openai_token_success() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response = serde_json::json!({
        "access_token": "oai-new-access",
        "refresh_token": "oai-new-refresh",
        "expires_in": 7200
    });
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let config = OAuthConfig::with_base_url(&server.uri());
    let result = refresh_openai_token(&config, "old-refresh").await.unwrap();
    assert_eq!(result.access_token, "oai-new-access");
    assert_eq!(result.refresh_token, Some("oai-new-refresh".to_string()));
}

#[tokio::test]
async fn test_refresh_openai_token_error_status() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server)
        .await;

    let config = OAuthConfig::with_base_url(&server.uri());
    let err = refresh_openai_token(&config, "bad").await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("401"));
    assert!(msg.contains("OpenAI token refresh failed"));
}

#[tokio::test]
async fn test_exchange_anthropic_code_error_status() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
        .mount(&server)
        .await;

    let config = OAuthConfig::with_base_url(&server.uri());
    let err = exchange_anthropic_code(&config, "code#state", "verifier")
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("403"));
    assert!(msg.contains("token exchange failed"));
}

#[tokio::test]
async fn test_exchange_anthropic_code_no_hash_in_input() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response = serde_json::json!({
        "access_token": "sk-ant-oat01-nohash",
        "expires_in": 28800
    });
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let config = OAuthConfig::with_base_url(&server.uri());
    // No '#': state defaults to "" via parts.get(1)
    let result = exchange_anthropic_code(&config, "rawcodeonly", "verifier")
        .await
        .unwrap();
    assert_eq!(result.access_token, "sk-ant-oat01-nohash");
}

// ===================================================================
// Parse-error arms (HTTP 200 but body is not valid token JSON)
// ===================================================================

#[tokio::test]
async fn test_request_device_code_parse_error() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/device/code"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json at all"))
        .mount(&server)
        .await;

    let config = OAuthConfig::with_base_url(&server.uri());
    let err = request_device_code(&config).await.unwrap_err();
    assert!(
        err.to_string().contains("parse device code response"),
        "got: {err}"
    );
}

#[tokio::test]
async fn test_exchange_anthropic_code_parse_error() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{ broken"))
        .mount(&server)
        .await;

    let config = OAuthConfig::with_base_url(&server.uri());
    let err = exchange_anthropic_code(&config, "code#state", "verifier")
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("parse token response"),
        "got: {err}"
    );
}

#[tokio::test]
async fn test_refresh_anthropic_token_parse_error() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string("nope"))
        .mount(&server)
        .await;

    let config = OAuthConfig::with_base_url(&server.uri());
    let err = refresh_anthropic_token(&config, "rt").await.unwrap_err();
    assert!(
        err.to_string().contains("parse refresh response"),
        "got: {err}"
    );
}

#[tokio::test]
async fn test_refresh_openai_token_parse_error() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<<<"))
        .mount(&server)
        .await;

    let config = OAuthConfig::with_base_url(&server.uri());
    let err = refresh_openai_token(&config, "rt").await.unwrap_err();
    assert!(
        err.to_string().contains("parse refresh response"),
        "got: {err}"
    );
}

// ===================================================================
// Request-failed arms (transport error: connection refused on a closed
// localhost port — no live network, fails fast and deterministically).
// ===================================================================

/// Reserve, then immediately release, an ephemeral localhost port so that a
/// connection to it is refused. No server ever listens here.
fn closed_localhost_base_url() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    format!("http://{}", addr)
}

#[tokio::test]
async fn test_request_device_code_transport_error() {
    let config = OAuthConfig::with_base_url(&closed_localhost_base_url());
    let err = request_device_code(&config).await.unwrap_err();
    assert!(
        err.to_string().contains("device code request failed"),
        "got: {err}"
    );
}

#[tokio::test]
async fn test_exchange_anthropic_code_transport_error() {
    let config = OAuthConfig::with_base_url(&closed_localhost_base_url());
    let err = exchange_anthropic_code(&config, "code#state", "verifier")
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("token exchange request failed"),
        "got: {err}"
    );
}

#[tokio::test]
async fn test_refresh_anthropic_token_transport_error() {
    let config = OAuthConfig::with_base_url(&closed_localhost_base_url());
    let err = refresh_anthropic_token(&config, "rt").await.unwrap_err();
    assert!(
        err.to_string().contains("token refresh request failed"),
        "got: {err}"
    );
}

#[tokio::test]
async fn test_exchange_openai_code_transport_error() {
    let config = OAuthConfig::with_base_url(&closed_localhost_base_url());
    let err = exchange_openai_code(&config, "code", "verifier")
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("token exchange request failed"),
        "got: {err}"
    );
}

#[tokio::test]
async fn test_refresh_openai_token_transport_error() {
    let config = OAuthConfig::with_base_url(&closed_localhost_base_url());
    let err = refresh_openai_token(&config, "rt").await.unwrap_err();
    assert!(
        err.to_string().contains("token refresh request failed"),
        "got: {err}"
    );
}

#[tokio::test]
async fn test_exchange_anthropic_code_without_refresh_token() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response = serde_json::json!({
        "access_token": "sk-ant-oat01-exchanged",
        "expires_in": 28800
    });

    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response))
        .mount(&server)
        .await;

    let config = OAuthConfig::with_base_url(&server.uri());
    let result = exchange_anthropic_code(&config, "code123#state456", "verifier")
        .await
        .unwrap();
    assert_eq!(result.access_token, "sk-ant-oat01-exchanged");
    assert_eq!(result.refresh_token, None);
}
