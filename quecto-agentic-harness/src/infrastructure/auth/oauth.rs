// OAuth: OAuth 2.0 and device-code login flows for OpenAI and Anthropic.

use crate::domain::error::DomainError;
use serde::Deserialize;

/// Overall request timeout for OAuth HTTP calls.
const OAUTH_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Connect timeout for OAuth HTTP calls (#811). A cold or unreachable token
/// endpoint must fail fast instead of blocking for the full request timeout.
/// Token refresh is lazy (refresh-on-401, after the socket announce), but a slow
/// endpoint at first use should surface quickly rather than stall the request.
const OAUTH_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Build the shared HTTP client used for every OAuth flow (device code,
/// authorization-code exchange, and token refresh). Centralises the request and
/// connect timeouts so no OAuth path can regress to a missing `connect_timeout`.
fn oauth_http_client() -> Result<reqwest::Client, DomainError> {
    reqwest::Client::builder()
        .timeout(OAUTH_REQUEST_TIMEOUT)
        .connect_timeout(OAUTH_CONNECT_TIMEOUT)
        .build()
        .map_err(|e| DomainError::Provider(format!("failed to build HTTP client: {}", e)))
}

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
pub fn build_anthropic_auth_url(config: &OAuthConfig, pkce: &PkceCodes, state: &str) -> String {
    let params = [
        ("code", "true"),
        ("client_id", &config.client_id),
        ("response_type", "code"),
        ("redirect_uri", &config.redirect_uri),
        ("scope", &config.scopes),
        ("code_challenge", &pkce.challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
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

    let client = oauth_http_client()?;

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
        let _ = discard_error_body(resp).await;
        return Err(DomainError::Provider(format!(
            "token exchange failed ({})",
            status
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
    let client = oauth_http_client()?;

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
        let _ = discard_error_body(resp).await;
        return Err(DomainError::Provider(format!(
            "token refresh failed ({})",
            status
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
        ("originator", "codex_cli_rs"),
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
    let client = oauth_http_client()?;

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
        let _ = discard_error_body(resp).await;
        return Err(DomainError::Provider(format!(
            "OpenAI token exchange failed ({})",
            status
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
    let client = oauth_http_client()?;

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
        let _ = discard_error_body(resp).await;
        return Err(DomainError::Provider(format!(
            "OpenAI token refresh failed ({})",
            status
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

        // Parse query params (URL-decode values to handle encoded chars)
        let query = path.split('?').nth(1).unwrap_or("");
        let params: std::collections::HashMap<String, String> = query
            .split('&')
            .filter_map(|p| {
                let (k, v) = p.split_once('=')?;
                Some((
                    k.to_string(),
                    urlencoding::decode(v).unwrap_or_default().into_owned(),
                ))
            })
            .collect();

        let state = params.get("state").map(|s| s.as_str()).unwrap_or("");
        let code = params.get("code").map(|s| s.as_str()).unwrap_or("");

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

/// Response from an OAuth token exchange or refresh.
///
/// Per RFC 6749 §5.1, `refresh_token` is OPTIONAL. Some servers omit it
/// in refresh responses, meaning "keep using the old refresh token."
#[derive(Debug, Deserialize)]
pub struct OAuthTokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
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

/// Drop an OAuth error response without reading its body. Never buffering the
/// body — regardless of the server's `Content-Length`, transfer encoding, or
/// frame sizes — is the only strict way to guarantee the CLI does not read an
/// unbounded amount from a malicious or misbehaving server. Error bodies may
/// also contain provider internals or secrets, so callers report only the
/// HTTP status, never response details.
async fn discard_error_body(_resp: reqwest::Response) {}

/// Initiate a device code flow: POST to the device code endpoint and
/// return the response containing the user code and verification URI.
pub async fn request_device_code(config: &OAuthConfig) -> Result<DeviceCodeResponse, DomainError> {
    let client = oauth_http_client()?;

    let resp = client
        .post(&config.device_code_url)
        .form(&[("client_id", &config.client_id)])
        .send()
        .await
        .map_err(|e| DomainError::Provider(format!("device code request failed: {}", e)))?;

    let status = resp.status().as_u16();
    if status != 200 {
        // Drop the response without reading its body. Do NOT include server
        // response details in the error message to avoid leaking internals.
        let _ = discard_error_body(resp).await;
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
#[path = "oauth_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "oauth_error_body_tests.rs"]
mod error_body_tests;
