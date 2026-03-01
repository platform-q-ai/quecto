// OAuth: OAuth 2.0 and device-code login flows for OpenAI and Anthropic.

use crate::domain::error::DomainError;
use serde::Deserialize;

/// Provider-specific OAuth configuration.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub authorization_url: String,
    pub device_code_url: String,
    pub token_url: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub scopes: String,
}

impl OAuthConfig {
    /// Return the OAuth config for a known provider.
    pub fn for_provider(provider: &str) -> Option<Self> {
        match provider {
            "openai" => Some(Self {
                authorization_url: "https://auth.openai.com/oauth/authorize".into(),
                device_code_url: "https://auth.openai.com/device/code".into(),
                token_url: "https://auth.openai.com/oauth/token".into(),
                client_id: "app_EMoamEEZ73f0CkXaXp7hrann".into(),
                redirect_uri: "http://localhost:1455/auth/callback".into(),
                scopes: "openid profile email offline_access".into(),
            }),
            "anthropic" => Some(Self {
                authorization_url: "https://claude.ai/oauth/authorize".into(),
                device_code_url: String::new(), // Anthropic doesn't use device code
                token_url: "https://console.anthropic.com/v1/oauth/token".into(),
                client_id: "9d1c250a-e61b-44d9-88ed-5944d1962f5e".into(),
                redirect_uri: "https://console.anthropic.com/oauth/code/callback".into(),
                scopes: "org:create_api_key user:profile user:inference".into(),
            }),
            _ => None,
        }
    }

    /// Return the OAuth config with custom base URLs (for testing).
    pub fn with_base_url(base_url: &str) -> Self {
        Self {
            authorization_url: format!("{}/authorize", base_url),
            device_code_url: format!("{}/device/code", base_url),
            token_url: format!("{}/oauth/token", base_url),
            client_id: "quecto-cli".into(),
            redirect_uri: format!("{}/callback", base_url),
            scopes: "test:scope".into(),
        }
    }
}

/// PKCE (Proof Key for Code Exchange) codes for OAuth.
#[derive(Debug, Clone)]
pub struct PkceCodes {
    pub verifier: String,
    pub challenge: String,
}

/// Generate a PKCE verifier and challenge pair.
pub fn generate_pkce() -> PkceCodes {
    use base64::Engine;
    use rand::RngCore;
    use sha2::Digest;

    // Generate 32 random bytes for the verifier
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf);

    // SHA-256 hash of the verifier, base64url-encoded
    let mut hasher = sha2::Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash);

    PkceCodes {
        verifier,
        challenge,
    }
}

/// Build the Anthropic OAuth authorization URL with PKCE.
pub fn build_anthropic_auth_url(config: &OAuthConfig, pkce: &PkceCodes) -> String {
    let params = [
        ("code", "true"),
        ("client_id", &config.client_id),
        ("response_type", "code"),
        ("redirect_uri", &config.redirect_uri),
        ("scope", &config.scopes),
        ("code_challenge", &pkce.challenge),
        ("code_challenge_method", "S256"),
        ("state", &pkce.verifier),
    ];

    let query: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
        .collect();

    format!("{}?{}", config.authorization_url, query.join("&"))
}

/// Exchange an authorization code for OAuth tokens (Anthropic).
///
/// The `auth_code` should be the raw string pasted by the user, in
/// the format `code#state` as returned by Anthropic's callback.
pub async fn exchange_anthropic_code(
    config: &OAuthConfig,
    auth_code: &str,
    pkce_verifier: &str,
) -> Result<OAuthTokenResponse, DomainError> {
    let parts: Vec<&str> = auth_code.splitn(2, '#').collect();
    let code = parts[0];
    let state = parts.get(1).copied().unwrap_or("");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| DomainError::Provider(format!("failed to build HTTP client: {}", e)))?;

    let body = serde_json::json!({
        "grant_type": "authorization_code",
        "client_id": config.client_id,
        "code": code,
        "state": state,
        "redirect_uri": config.redirect_uri,
        "code_verifier": pkce_verifier,
    });

    let resp = client
        .post(&config.token_url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| DomainError::Provider(format!("token exchange request failed: {}", e)))?;

    let status = resp.status().as_u16();
    if status != 200 {
        let error_body = resp.text().await.unwrap_or_default();
        return Err(DomainError::Provider(format!(
            "token exchange failed ({}): {}",
            status, error_body
        )));
    }

    resp.json::<OAuthTokenResponse>()
        .await
        .map_err(|e| DomainError::Provider(format!("failed to parse token response: {}", e)))
}

/// Refresh an Anthropic OAuth access token using a refresh token.
pub async fn refresh_anthropic_token(
    config: &OAuthConfig,
    refresh_token: &str,
) -> Result<OAuthTokenResponse, DomainError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| DomainError::Provider(format!("failed to build HTTP client: {}", e)))?;

    let body = serde_json::json!({
        "grant_type": "refresh_token",
        "client_id": config.client_id,
        "refresh_token": refresh_token,
    });

    let resp = client
        .post(&config.token_url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| DomainError::Provider(format!("token refresh request failed: {}", e)))?;

    let status = resp.status().as_u16();
    if status != 200 {
        let error_body = resp.text().await.unwrap_or_default();
        return Err(DomainError::Provider(format!(
            "token refresh failed ({}): {}",
            status, error_body
        )));
    }

    resp.json::<OAuthTokenResponse>()
        .await
        .map_err(|e| DomainError::Provider(format!("failed to parse refresh response: {}", e)))
}

/// Build the OpenAI OAuth authorization URL with PKCE.
pub fn build_openai_auth_url(config: &OAuthConfig, pkce: &PkceCodes, state: &str) -> String {
    let params = [
        ("response_type", "code"),
        ("client_id", &config.client_id),
        ("redirect_uri", &config.redirect_uri),
        ("scope", &config.scopes),
        ("code_challenge", &pkce.challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("id_token_add_organizations", "true"),
        ("codex_cli_simplified_flow", "true"),
        ("originator", "quecto"),
    ];

    let query: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
        .collect();

    format!("{}?{}", config.authorization_url, query.join("&"))
}

/// Generate a random hex state string for OAuth.
pub fn generate_state() -> String {
    use rand::RngCore;
    let mut buf = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Exchange an authorization code for OpenAI OAuth tokens.
///
/// OpenAI uses `application/x-www-form-urlencoded` for token exchange.
pub async fn exchange_openai_code(
    config: &OAuthConfig,
    code: &str,
    pkce_verifier: &str,
) -> Result<OAuthTokenResponse, DomainError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| DomainError::Provider(format!("failed to build HTTP client: {}", e)))?;

    let resp = client
        .post(&config.token_url)
        .form(&[
            ("grant_type", "authorization_code"),
            ("client_id", &config.client_id),
            ("code", code),
            ("code_verifier", pkce_verifier),
            ("redirect_uri", &config.redirect_uri),
        ])
        .send()
        .await
        .map_err(|e| DomainError::Provider(format!("token exchange request failed: {}", e)))?;

    let status = resp.status().as_u16();
    if status != 200 {
        let error_body = resp.text().await.unwrap_or_default();
        return Err(DomainError::Provider(format!(
            "OpenAI token exchange failed ({}): {}",
            status, error_body
        )));
    }

    resp.json::<OAuthTokenResponse>()
        .await
        .map_err(|e| DomainError::Provider(format!("failed to parse token response: {}", e)))
}

/// Refresh an OpenAI OAuth access token using a refresh token.
pub async fn refresh_openai_token(
    config: &OAuthConfig,
    refresh_token: &str,
) -> Result<OAuthTokenResponse, DomainError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| DomainError::Provider(format!("failed to build HTTP client: {}", e)))?;

    let resp = client
        .post(&config.token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", &config.client_id),
        ])
        .send()
        .await
        .map_err(|e| DomainError::Provider(format!("token refresh request failed: {}", e)))?;

    let status = resp.status().as_u16();
    if status != 200 {
        let error_body = resp.text().await.unwrap_or_default();
        return Err(DomainError::Provider(format!(
            "OpenAI token refresh failed ({}): {}",
            status, error_body
        )));
    }

    resp.json::<OAuthTokenResponse>()
        .await
        .map_err(|e| DomainError::Provider(format!("failed to parse refresh response: {}", e)))
}

/// Start a local HTTP server to receive the OAuth callback and return the authorization code.
///
/// Listens on `127.0.0.1:1455` and waits up to 5 minutes for the callback.
/// Returns the code from the `?code=` query parameter if the state matches.
pub async fn wait_for_oauth_callback(
    expected_state: &str,
    timeout_secs: u64,
) -> Result<String, DomainError> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:1455")
        .await
        .map_err(|e| DomainError::Provider(format!("failed to bind callback server: {}", e)))?;

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    let expected = expected_state.to_string();

    loop {
        let accept = tokio::time::timeout_at(deadline, listener.accept()).await;
        let (mut stream, _) = match accept {
            Ok(Ok(conn)) => conn,
            Ok(Err(e)) => {
                return Err(DomainError::Provider(format!(
                    "callback accept error: {}",
                    e
                )));
            }
            Err(_) => {
                return Err(DomainError::Provider(
                    "OAuth callback timed out".to_string(),
                ));
            }
        };

        let mut buf = vec![0u8; 4096];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]);

        // Parse the GET /auth/callback?code=...&state=... line
        let first_line = request.lines().next().unwrap_or("");
        let path = first_line.split_whitespace().nth(1).unwrap_or("");

        if !path.starts_with("/auth/callback") {
            let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nNot found";
            let _ = stream.write_all(resp.as_bytes()).await;
            continue;
        }

        // Parse query params
        let query = path.split('?').nth(1).unwrap_or("");
        let params: std::collections::HashMap<&str, &str> =
            query.split('&').filter_map(|p| p.split_once('=')).collect();

        let state = params.get("state").copied().unwrap_or("");
        let code = params.get("code").copied().unwrap_or("");

        if state != expected {
            let resp = "HTTP/1.1 400 Bad Request\r\nContent-Length: 14\r\n\r\nState mismatch";
            let _ = stream.write_all(resp.as_bytes()).await;
            continue;
        }

        if code.is_empty() {
            let resp = "HTTP/1.1 400 Bad Request\r\nContent-Length: 12\r\n\r\nMissing code";
            let _ = stream.write_all(resp.as_bytes()).await;
            continue;
        }

        let html = "<html><body><h2>Authentication successful!</h2><p>You can close this window.</p></body></html>";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
            html.len(),
            html
        );
        let _ = stream.write_all(resp.as_bytes()).await;

        return Ok(code.to_string());
    }
}

/// Extract the `chatgpt_account_id` from an OpenAI JWT access token.
///
/// Returns `None` if the token is not a JWT or doesn't contain the account ID.
pub fn extract_openai_account_id(token: &str) -> Option<String> {
    use base64::Engine;

    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return None;
    }

    // Base64url decode the payload (add padding as needed)
    let payload = parts[1];
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .ok()?;

    let claims: serde_json::Value = serde_json::from_slice(&decoded).ok()?;

    // Try nested path: https://api.openai.com/auth -> chatgpt_account_id
    if let Some(account_id) = claims
        .get("https://api.openai.com/auth")
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(|v| v.as_str())
    {
        if !account_id.is_empty() {
            return Some(account_id.to_string());
        }
    }

    None
}

/// Check if a token is an OpenAI OAuth JWT (has three dot-separated parts).
pub fn is_openai_oauth_token(token: &str) -> bool {
    // OpenAI OAuth tokens are JWTs (eyJ...), standard API keys start with sk-
    token.starts_with("eyJ") && token.split('.').count() == 3
}

/// Response from an OAuth token exchange or refresh.
#[derive(Debug, Deserialize)]
pub struct OAuthTokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
}

/// Response from a device code grant request.
///
/// Note: `Debug` is intentionally NOT derived — `device_code` is a secret
/// that should not appear in logs. Use explicit field access instead.
#[derive(Deserialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
}

impl std::fmt::Debug for DeviceCodeResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceCodeResponse")
            .field("device_code", &"[REDACTED]")
            .field("user_code", &self.user_code)
            .field("verification_uri", &self.verification_uri)
            .finish()
    }
}

/// Maximum error body size to read from OAuth server responses (4 KB).
const MAX_ERROR_BODY_BYTES: usize = 4096;

/// Initiate a device code flow: POST to the device code endpoint and
/// return the response containing the user code and verification URI.
pub async fn request_device_code(config: &OAuthConfig) -> Result<DeviceCodeResponse, DomainError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| DomainError::Provider(format!("failed to build HTTP client: {}", e)))?;

    let resp = client
        .post(&config.device_code_url)
        .form(&[("client_id", &config.client_id)])
        .send()
        .await
        .map_err(|e| DomainError::Provider(format!("device code request failed: {}", e)))?;

    let status = resp.status().as_u16();
    if status != 200 {
        // Read and discard the body (truncated to prevent OOM from
        // malicious servers), but do NOT include server response
        // details in the error message to avoid leaking internals.
        let _ = resp
            .bytes()
            .await
            .map(|b| b.len().min(MAX_ERROR_BODY_BYTES));
        return Err(DomainError::Provider(format!(
            "device code request failed ({})",
            status
        )));
    }

    resp.json::<DeviceCodeResponse>()
        .await
        .map_err(|e| DomainError::Provider(format!("failed to parse device code response: {}", e)))
}

/// Check if a token is an Anthropic OAuth access token.
pub fn is_anthropic_oauth_token(token: &str) -> bool {
    token.starts_with("sk-ant-oat")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openai_config() {
        let config = OAuthConfig::for_provider("openai").unwrap();
        assert!(config.authorization_url.contains("openai.com"));
        assert!(config.device_code_url.contains("openai.com"));
        assert_eq!(config.client_id, "quecto-cli");
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
        let url = build_anthropic_auth_url(&config, &pkce);
        assert!(url.starts_with("https://claude.ai/oauth/authorize?"));
        assert!(url.contains("client_id="));
        assert!(url.contains("code_challenge=test-challenge"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("redirect_uri="));
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
        assert_eq!(result.refresh_token, "sk-ant-ort01-test-refresh");
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
        assert_eq!(result.refresh_token, "sk-ant-ort01-new-refresh");
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
}
