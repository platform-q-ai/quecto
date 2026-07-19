use super::*;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[test]
fn oauth_http_client_builds_and_token_debug_redacts_secrets() {
    let _client = oauth_http_client().unwrap();
    let response = OAuthTokenResponse {
        access_token: "access-secret".into(),
        refresh_token: Some("refresh-secret".into()),
        expires_in: 3600,
    };
    let rendered = format!("{response:?}");
    assert!(rendered.contains("<redacted>"));
    assert!(rendered.contains("expires_in"));
    assert!(!rendered.contains("access-secret"));
    assert!(!rendered.contains("refresh-secret"));
}

#[test]
fn parse_loopback_redirect_accepts_loopback_and_rejects_remote() {
    assert_eq!(
        parse_loopback_redirect("http://127.0.0.1:56121/callback?code=x").unwrap(),
        ("127.0.0.1:56121".to_string(), "/callback".to_string())
    );
    assert_eq!(
        parse_loopback_redirect("http://localhost:1455/auth/callback").unwrap(),
        ("localhost:1455".to_string(), "/auth/callback".to_string())
    );
    assert_eq!(
        parse_loopback_redirect("http://[::1]:1455/?ignored=yes").unwrap(),
        ("[::1]:1455".to_string(), "/".to_string())
    );
    assert!(parse_loopback_redirect("https://127.0.0.1:1/cb").is_err());
    assert!(parse_loopback_redirect("http://example.com:1/cb").is_err());
    assert!(parse_loopback_redirect("http://127.0.0.1/cb").is_err());
}

#[tokio::test]
async fn refresh_xai_token_posts_form_and_parses_success() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=refresh-1"))
        .and(body_string_contains("client_id=client-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "access-2",
            "refresh_token": "refresh-2",
            "expires_in": 123
        })))
        .mount(&server)
        .await;

    let config = OAuthConfig {
        authorization_url: "http://unused/auth".into(),
        device_code_url: "http://unused/device".into(),
        token_url: format!("{}/token", server.uri()),
        client_id: "client-1".into(),
        redirect_uri: "http://127.0.0.1:1/cb".into(),
        scopes: String::new(),
    };
    let token = refresh_xai_token(&config, "refresh-1").await.unwrap();
    assert_eq!(token.access_token, "access-2");
    assert_eq!(token.refresh_token.as_deref(), Some("refresh-2"));
    assert_eq!(token.expires_in, 123);
}

#[tokio::test]
async fn wait_for_oauth_callback_zero_timeout_errors_without_hanging() {
    let err = wait_for_oauth_callback("state", 0)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("timed out") || err.contains("address already in use"));
}

#[tokio::test]
async fn refresh_xai_token_error_discards_body_and_reports_status_only() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(401).set_body_string("secret-provider-body"))
        .mount(&server)
        .await;
    let config = OAuthConfig {
        authorization_url: "http://unused/auth".into(),
        device_code_url: "http://unused/device".into(),
        token_url: format!("{}/token", server.uri()),
        client_id: "client-1".into(),
        redirect_uri: "http://127.0.0.1:1/cb".into(),
        scopes: String::new(),
    };

    let err = refresh_xai_token(&config, "refresh-secret")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("xAI token refresh failed (401)"), "{err}");
    assert!(!err.contains("secret-provider-body"), "{err}");
}

#[tokio::test]
async fn refresh_xai_token_invalid_json_reports_parse_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not-json"))
        .mount(&server)
        .await;
    let config = OAuthConfig {
        authorization_url: "http://unused/auth".into(),
        device_code_url: "http://unused/device".into(),
        token_url: format!("{}/token", server.uri()),
        client_id: "client-1".into(),
        redirect_uri: "http://127.0.0.1:1/cb".into(),
        scopes: String::new(),
    };

    let err = refresh_xai_token(&config, "refresh-secret")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("failed to parse refresh response"), "{err}");
}
