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

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_oauth_parse_errors_are_reported() {
    type Call = Box<
        dyn for<'a> FnOnce(
            &'a OAuthConfig,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(), DomainError>> + Send + 'a>,
        >,
    >;

    let cases: [(&'static str, Call); 4] = [
        (
            "/device/code",
            Box::new(|config| {
                Box::pin(async move { request_device_code(config).await.map(|_| ()) })
            }),
        ),
        (
            "/oauth/token",
            Box::new(|config| {
                Box::pin(async move {
                    exchange_anthropic_code(config, "code#state", "verifier")
                        .await
                        .map(|_| ())
                })
            }),
        ),
        (
            "/oauth/token",
            Box::new(|config| {
                Box::pin(async move { refresh_anthropic_token(config, "rt").await.map(|_| ()) })
            }),
        ),
        (
            "/oauth/token",
            Box::new(|config| {
                Box::pin(async move { refresh_openai_token(config, "rt").await.map(|_| ()) })
            }),
        ),
    ];

    for (endpoint, call) in cases {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(endpoint))
            .respond_with(ResponseTemplate::new(200).set_body_string("not valid json"))
            .mount(&server)
            .await;

        let config = OAuthConfig::with_base_url(&server.uri());
        let err = call(&config).await.unwrap_err();
        assert!(
            err.to_string().contains("parse"),
            "endpoint {endpoint}: expected parse error, got: {err}"
        );
    }
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
async fn test_oauth_transport_errors_are_reported() {
    type Call = Box<
        dyn for<'a> FnOnce(
            &'a OAuthConfig,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(), DomainError>> + Send + 'a>,
        >,
    >;

    let cases: [(&'static str, Call); 5] = [
        (
            "device code request failed",
            Box::new(|config| {
                Box::pin(async move { request_device_code(config).await.map(|_| ()) })
            }),
        ),
        (
            "token exchange request failed",
            Box::new(|config| {
                Box::pin(async move {
                    exchange_anthropic_code(config, "code#state", "verifier")
                        .await
                        .map(|_| ())
                })
            }),
        ),
        (
            "token refresh request failed",
            Box::new(|config| {
                Box::pin(async move { refresh_anthropic_token(config, "rt").await.map(|_| ()) })
            }),
        ),
        (
            "token exchange request failed",
            Box::new(|config| {
                Box::pin(async move {
                    exchange_openai_code(config, "code", "verifier")
                        .await
                        .map(|_| ())
                })
            }),
        ),
        (
            "token refresh request failed",
            Box::new(|config| {
                Box::pin(async move { refresh_openai_token(config, "rt").await.map(|_| ()) })
            }),
        ),
    ];

    let base_url = closed_localhost_base_url();
    for (expected, call) in cases {
        let config = OAuthConfig::with_base_url(&base_url);
        let err = call(&config).await.unwrap_err();
        assert!(
            err.to_string().contains(expected),
            "expected '{expected}' in error, got: {err}"
        );
    }
}

/// Issue #811: every OAuth flow shares one HTTP client (`oauth_http_client`)
/// that MUST configure a short `connect_timeout` so a cold/unreachable token
/// endpoint fails fast instead of blocking for the full 30s overall timeout.
///
/// This is the deterministic, network-free guard: it asserts the configured
/// connect timeout sits in a fast band (1-5s) and is strictly shorter than the
/// overall request timeout. Because `refresh_anthropic_token`,
/// `refresh_openai_token`, both code-exchange flows, and the device-code flow
/// all build their client via `oauth_http_client()`, this single assertion
/// covers the connect-timeout criterion for ALL OAuth refresh paths — there is
/// no path that can regress to a missing `connect_timeout`.
#[test]
fn test_oauth_connect_timeout_is_fast() {
    assert!(
        OAUTH_CONNECT_TIMEOUT >= std::time::Duration::from_secs(1)
            && OAUTH_CONNECT_TIMEOUT <= std::time::Duration::from_secs(5),
        "OAuth connect_timeout must be a short 1-5s band so a cold endpoint fails \
         fast; got {OAUTH_CONNECT_TIMEOUT:?}"
    );
    assert!(
        OAUTH_CONNECT_TIMEOUT < OAUTH_REQUEST_TIMEOUT,
        "connect_timeout ({OAUTH_CONNECT_TIMEOUT:?}) must be shorter than the \
         overall request timeout ({OAUTH_REQUEST_TIMEOUT:?})"
    );
    // The shared builder must succeed (it is the only client constructor for
    // every OAuth path).
    assert!(oauth_http_client().is_ok());
}

/// Issue #811 (behavioural cross-check): connecting to a reserved TEST-NET-1
/// address (192.0.2.1, RFC 5737) is blackholed, so the connect hangs until the
/// 3s `connect_timeout` fires — well under the 30s overall timeout, but not
/// instantly. Both the Anthropic and OpenAI refresh paths are exercised.
///
/// When outbound networking is unavailable (sandboxed/offline CI), the connect
/// fails immediately rather than hanging; the wall-clock signal is then
/// meaningless, so the timing assertion self-skips instead of producing a false
/// pass (an instant failure must not be misread as the timeout working).
#[tokio::test]
async fn test_refresh_paths_fail_fast_via_connect_timeout() {
    let cfg = OAuthConfig::with_base_url("http://192.0.2.1:81");
    let assert_fast = |label: &str, err: DomainError, elapsed: std::time::Duration| {
        assert!(
            err.to_string().contains("token refresh request failed"),
            "{label}: expected a transport error, got: {err}"
        );
        if elapsed < std::time::Duration::from_millis(1500) {
            return; // offline: connect failed instantly, timing not meaningful
        }
        assert!(
            elapsed < std::time::Duration::from_secs(8),
            "{label}: must fail fast via connect_timeout, not the 30s overall \
             timeout (took {elapsed:?})"
        );
    };

    let start = std::time::Instant::now();
    let err = refresh_anthropic_token(&cfg, "rt").await.unwrap_err();
    assert_fast("anthropic", err, start.elapsed());

    let start = std::time::Instant::now();
    let err = refresh_openai_token(&cfg, "rt").await.unwrap_err();
    assert_fast("openai", err, start.elapsed());
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
