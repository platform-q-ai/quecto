// Tests for the xAI (Grok) OAuth flow and the generalized callback listener.

use super::*;

// --- OAuthConfig ---

#[test]
fn test_xai_config() {
    let config = OAuthConfig::for_provider("xai").unwrap();
    assert_eq!(
        config.authorization_url,
        "https://auth.x.ai/oauth2/authorize"
    );
    assert_eq!(config.token_url, "https://auth.x.ai/oauth2/token");
    assert_eq!(config.client_id, "b1a00492-073a-47ea-816f-4c329264a828");
    assert_eq!(config.redirect_uri, "http://127.0.0.1:56121/callback");
    assert_eq!(
        config.scopes,
        "openid profile email offline_access grok-cli:access api:access"
    );
}

#[test]
fn test_xai_device_code_deliberately_unsupported() {
    // The generic device-code command does not poll or persist credentials
    // yet; exposing an endpoint would report success without logging in.
    let config = OAuthConfig::for_provider("xai").unwrap();
    assert!(config.device_code_url.is_empty());
}

// --- Authorization URL ---

fn parsed_query(url: &str) -> std::collections::HashMap<String, String> {
    let query = url.split('?').nth(1).expect("auth url has a query");
    query
        .split('&')
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((
                k.to_string(),
                urlencoding::decode(v).unwrap_or_default().into_owned(),
            ))
        })
        .collect()
}

#[test]
fn test_build_xai_auth_url() {
    let config = OAuthConfig::for_provider("xai").unwrap();
    let pkce = PkceCodes {
        verifier: "test-verifier".into(),
        challenge: "test-challenge".into(),
    };
    let url = build_xai_auth_url(&config, &pkce, "test-state");

    assert!(url.starts_with("https://auth.x.ai/oauth2/authorize?"));
    let q = parsed_query(&url);
    assert_eq!(q["response_type"], "code");
    assert_eq!(q["client_id"], config.client_id);
    assert_eq!(q["redirect_uri"], config.redirect_uri);
    assert_eq!(q["scope"], config.scopes);
    assert_eq!(q["code_challenge"], "test-challenge");
    assert_eq!(q["code_challenge_method"], "S256");
    assert_eq!(q["state"], "test-state");
    assert_eq!(q["plan"], "generic");
    assert_eq!(q["referrer"], "quecto");
}

// --- Token exchange ---

fn config_with_token_url(token_url: String) -> OAuthConfig {
    let mut config = OAuthConfig::for_provider("xai").unwrap();
    config.token_url = token_url;
    config
}

#[tokio::test]
async fn test_exchange_xai_code_success() {
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code=the-code"))
        .and(body_string_contains("code_verifier=the-verifier"))
        .and(body_string_contains(
            "client_id=b1a00492-073a-47ea-816f-4c329264a828",
        ))
        .and(body_string_contains(
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A56121%2Fcallback",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "at-1",
            "refresh_token": "rt-1",
            "expires_in": 3600
        })))
        .mount(&server)
        .await;

    let config = config_with_token_url(format!("{}/token", server.uri()));
    let resp = exchange_xai_code(&config, "the-code", "the-verifier")
        .await
        .unwrap();
    assert_eq!(resp.access_token, "at-1");
    assert_eq!(resp.refresh_token.as_deref(), Some("rt-1"));
}

#[tokio::test]
async fn test_exchange_xai_code_success_without_refresh_token() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "at-only",
            "expires_in": 3600
        })))
        .mount(&server)
        .await;

    let config = config_with_token_url(format!("{}/token", server.uri()));
    let resp = exchange_xai_code(&config, "c", "v").await.unwrap();
    assert_eq!(resp.access_token, "at-only");
    assert!(resp.refresh_token.is_none());
}

#[tokio::test]
async fn test_exchange_xai_code_error_status() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(401).set_body_string("secret detail"))
        .mount(&server)
        .await;

    let config = config_with_token_url(format!("{}/token", server.uri()));
    let err = exchange_xai_code(&config, "c", "v").await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("401"), "got: {}", msg);
    // Provider error bodies must not leak into user-facing errors.
    assert!(!msg.contains("secret detail"), "got: {}", msg);
}

#[tokio::test]
async fn test_exchange_xai_code_malformed_json() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
        .mount(&server)
        .await;

    let config = config_with_token_url(format!("{}/token", server.uri()));
    assert!(exchange_xai_code(&config, "c", "v").await.is_err());
}

#[tokio::test]
async fn test_exchange_xai_code_connection_failure() {
    // Unroutable local port: nothing is listening.
    let config = config_with_token_url("http://127.0.0.1:1/token".into());
    assert!(exchange_xai_code(&config, "c", "v").await.is_err());
}

// --- Refresh ---

#[tokio::test]
async fn test_refresh_xai_token_success() {
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=rt-old"))
        .and(body_string_contains(
            "client_id=b1a00492-073a-47ea-816f-4c329264a828",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "at-2",
            "refresh_token": "rt-2",
            "expires_in": 3600
        })))
        .mount(&server)
        .await;

    let config = config_with_token_url(format!("{}/token", server.uri()));
    let resp = refresh_xai_token(&config, "rt-old").await.unwrap();
    assert_eq!(resp.access_token, "at-2");
    assert_eq!(resp.refresh_token.as_deref(), Some("rt-2"));
}

#[tokio::test]
async fn test_refresh_xai_token_error_status() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(400))
        .mount(&server)
        .await;

    let config = config_with_token_url(format!("{}/token", server.uri()));
    let err = refresh_xai_token(&config, "rt").await.unwrap_err();
    assert!(err.to_string().contains("400"));
}

// --- parse_loopback_redirect ---

#[test]
fn test_parse_loopback_redirect_xai() {
    let (addr, path) = parse_loopback_redirect("http://127.0.0.1:56121/callback").unwrap();
    assert_eq!(addr, "127.0.0.1:56121");
    assert_eq!(path, "/callback");
}

#[test]
fn test_parse_loopback_redirect_rejects_non_loopback() {
    assert!(parse_loopback_redirect("http://attacker.example/callback").is_err());
    assert!(parse_loopback_redirect("https://127.0.0.1:56121/callback").is_err());
}

#[test]
fn test_parse_loopback_redirect_supports_ipv6() {
    // Regression (PR #1087 follow-up): the old manual `:`-split parser made
    // the [::1] branch unreachable. With url::Url it must work.
    let (addr, path) = parse_loopback_redirect("http://[::1]:56121/callback").unwrap();
    assert_eq!(addr, "[::1]:56121");
    assert_eq!(path, "/callback");
    // And the bind address must parse as a real SocketAddr.
    assert!(addr.parse::<std::net::SocketAddr>().is_ok());
}

#[test]
fn test_parse_loopback_redirect_supports_localhost() {
    let (addr, path) = parse_loopback_redirect("http://localhost:1455/auth/callback").unwrap();
    assert_eq!(addr, "localhost:1455");
    assert_eq!(path, "/auth/callback");
}

#[test]
fn test_parse_loopback_redirect_rejects_lookalike_host() {
    // A subdomain that merely *contains* a loopback label must be rejected.
    assert!(parse_loopback_redirect("http://127.0.0.1.attacker.example:80/callback").is_err());
    assert!(parse_loopback_redirect("http://notlocalhost:1455/callback").is_err());
}

#[test]
fn test_parse_loopback_redirect_rejects_malformed() {
    assert!(parse_loopback_redirect("not a url").is_err());
    // Missing port -> we cannot bind a listener deterministically.
    assert!(parse_loopback_redirect("http://127.0.0.1/callback").is_err());
}

// --- Callback listener (wait_for_oauth_callback_at) ---

/// Bind an ephemeral port and return (addr, join-handle resolving to the result).
fn spawn_listener(
    state: &str,
    timeout_secs: u64,
) -> (String, tokio::task::JoinHandle<Result<String, DomainError>>) {
    // Bind first with std to learn a free port, then release it for the
    // helper. Small race, acceptable in tests.
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = probe.local_addr().unwrap().to_string();
    drop(probe);
    let state = state.to_string();
    let addr_clone = addr.clone();
    let handle = tokio::spawn(async move {
        wait_for_oauth_callback_at(&addr_clone, "/callback", &state, timeout_secs).await
    });
    (addr, handle)
}

async fn send_request(addr: &str, target: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // Bounded retry: never hang CI if the listener lost the bind race.
    let mut stream = None;
    for _ in 0..200 {
        match tokio::net::TcpStream::connect(addr).await {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
        }
    }
    let mut stream = stream.expect("listener never became connectable");
    stream
        .write_all(format!("GET {} HTTP/1.1\r\nHost: x\r\n\r\n", target).as_bytes())
        .await
        .unwrap();
    let mut buf = Vec::new();
    let _ = stream.read_to_end(&mut buf).await;
    String::from_utf8_lossy(&buf).into_owned()
}

#[tokio::test]
async fn test_callback_success() {
    let (addr, handle) = spawn_listener("s1", 5);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let resp = send_request(&addr, "/callback?code=abc&state=s1").await;
    assert!(resp.contains("200 OK"));
    assert_eq!(handle.await.unwrap().unwrap(), "abc");
}

#[tokio::test]
async fn test_callback_rejects_state_mismatch_then_accepts_valid() {
    let (addr, handle) = spawn_listener("good", 5);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let resp = send_request(&addr, "/callback?code=evil&state=bad").await;
    assert!(resp.contains("400"));
    let resp = send_request(&addr, "/callback?code=ok&state=good").await;
    assert!(resp.contains("200 OK"));
    assert_eq!(handle.await.unwrap().unwrap(), "ok");
}

#[tokio::test]
async fn test_callback_exact_path_match() {
    let (addr, handle) = spawn_listener("s", 5);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    // Prefix attacks must 404, not match.
    let resp = send_request(&addr, "/callbackevil?code=x&state=s").await;
    assert!(resp.contains("404"));
    let resp = send_request(&addr, "/callback/extra?code=x&state=s").await;
    assert!(resp.contains("404"));
    let resp = send_request(&addr, "/callback?code=fine&state=s").await;
    assert!(resp.contains("200 OK"));
    assert_eq!(handle.await.unwrap().unwrap(), "fine");
}

#[tokio::test]
async fn test_callback_url_decodes_code() {
    let (addr, handle) = spawn_listener("s", 5);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let resp = send_request(&addr, "/callback?code=a%2Fb%3D&state=s").await;
    assert!(resp.contains("200 OK"));
    assert_eq!(handle.await.unwrap().unwrap(), "a/b=");
}

#[tokio::test]
async fn test_callback_oauth_error_terminates_immediately() {
    let (addr, handle) = spawn_listener("s", 30);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let start = std::time::Instant::now();
    let resp = send_request(&addr, "/callback?error=access_denied&state=s").await;
    assert!(resp.contains("400"));
    let err = handle.await.unwrap().unwrap_err();
    assert!(err.to_string().contains("access_denied"), "got: {}", err);
    // Must not wait out the 30s timeout.
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
}

#[tokio::test]
async fn test_callback_timeout_with_no_connection() {
    let (_addr, handle) = spawn_listener("s", 1);
    let err = handle.await.unwrap().unwrap_err();
    assert!(err.to_string().contains("timed out"));
}

#[tokio::test]
async fn test_callback_timeout_with_silent_connection() {
    // Regression (PR #1087 review): a client that connects and sends no
    // bytes must not block login past the deadline.
    let (addr, handle) = spawn_listener("s", 1);
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let _stream = tokio::net::TcpStream::connect(&addr).await.unwrap();
    // Send nothing; the listener must still time out.
    let start = std::time::Instant::now();
    let err = handle.await.unwrap().unwrap_err();
    assert!(err.to_string().contains("timed out"));
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
}

#[tokio::test]
async fn test_callback_bind_failure_on_occupied_port() {
    let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = occupied.local_addr().unwrap().to_string();
    let err = wait_for_oauth_callback_at(&addr, "/callback", "s", 1)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("failed to bind"));
}
