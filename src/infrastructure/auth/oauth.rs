// OAuth: OAuth 2.0 and device-code login flows for OpenAI and Anthropic.

use crate::domain::error::DomainError;
use serde::Deserialize;

/// Provider-specific OAuth configuration.
#[derive(Debug, Clone)]
pub struct OAuthConfig {
    pub authorization_url: String,
    pub device_code_url: String,
    pub client_id: String,
}

impl OAuthConfig {
    /// Return the OAuth config for a known provider.
    pub fn for_provider(provider: &str) -> Option<Self> {
        match provider {
            "openai" => Some(Self {
                authorization_url: "https://auth.openai.com/authorize".into(),
                device_code_url: "https://auth.openai.com/device/code".into(),
                client_id: "quecto-cli".into(),
            }),
            _ => None,
        }
    }

    /// Return the OAuth config with custom base URLs (for testing).
    pub fn with_base_url(base_url: &str) -> Self {
        Self {
            authorization_url: format!("{}/authorize", base_url),
            device_code_url: format!("{}/device/code", base_url),
            client_id: "quecto-cli".into(),
        }
    }
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
    fn test_unknown_provider_returns_none() {
        assert!(OAuthConfig::for_provider("groq").is_none());
    }

    #[test]
    fn test_custom_base_url() {
        let config = OAuthConfig::with_base_url("http://localhost:9999");
        assert_eq!(config.authorization_url, "http://localhost:9999/authorize");
        assert_eq!(config.device_code_url, "http://localhost:9999/device/code");
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
}
