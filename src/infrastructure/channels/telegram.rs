// Telegram adapter: implements Telegram bot message handling.

use crate::domain::error::DomainError;
use crate::infrastructure::config::TelegramConfig;

const DEFAULT_TELEGRAM_API_BASE: &str = "https://api.telegram.org";
const ALLOW_INSECURE_API_BASE_ENV: &str = "QUECTO_ALLOW_INSECURE_TELEGRAM_API_BASE";

/// A parsed incoming Telegram message.
#[derive(Debug, Clone)]
pub struct TelegramMessage {
    /// The text content of the message.
    pub text: String,
    /// The sender's Telegram user ID as a string.
    pub sender_id: String,
    /// The chat ID where the message was sent.
    pub chat_id: String,
}

/// Represents a raw Telegram "Update" object (simplified).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TelegramUpdate {
    #[serde(default)]
    pub update_id: i64,
    pub message: Option<TelegramUpdateMessage>,
}

/// The "message" field inside a Telegram update.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TelegramUpdateMessage {
    pub message_id: i64,
    pub from: Option<TelegramUser>,
    pub chat: TelegramChat,
    pub text: Option<String>,
    pub voice: Option<TelegramVoice>,
}

/// A Telegram voice message attachment.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TelegramVoice {
    pub file_id: String,
    pub duration: u32,
    pub file_size: Option<u64>,
}

/// A Telegram user.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TelegramUser {
    pub id: i64,
    pub first_name: Option<String>,
    pub username: Option<String>,
}

/// A Telegram chat.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TelegramChat {
    pub id: i64,
    #[serde(rename = "type")]
    pub chat_type: Option<String>,
}

/// Response wrapper for Telegram API calls.
#[derive(Debug, serde::Deserialize)]
struct TelegramApiResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

/// The Telegram channel adapter.
#[derive(Debug, Clone)]
pub struct TelegramChannel {
    token: String,
    allow_from: Vec<String>,
    enabled: bool,
    api_base: String,
    client: reqwest::Client,
}

impl TelegramChannel {
    fn allow_insecure_api_base() -> bool {
        matches!(
            std::env::var(ALLOW_INSECURE_API_BASE_ENV).ok().as_deref(),
            Some("1") | Some("true") | Some("TRUE") | Some("True")
        )
    }

    fn validate_api_base(raw: &str, allow_insecure: bool) -> Option<String> {
        let parsed = reqwest::Url::parse(raw).ok()?;

        if parsed.scheme() != "https" && !(allow_insecure && parsed.scheme() == "http") {
            return None;
        }
        parsed.host_str()?;
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return None;
        }
        if parsed.query().is_some() || parsed.fragment().is_some() {
            return None;
        }
        if parsed.path() != "/" && !parsed.path().is_empty() {
            return None;
        }
        if !allow_insecure && parsed.host_str() != Some("api.telegram.org") {
            return None;
        }

        Some(parsed.to_string().trim_end_matches('/').to_string())
    }

    /// Create a new Telegram channel from config.
    pub fn new(config: &TelegramConfig) -> Self {
        let allow_insecure = Self::allow_insecure_api_base();
        let requested_api_base = if config.api_base.trim().is_empty() {
            DEFAULT_TELEGRAM_API_BASE
        } else {
            config.api_base.trim()
        };
        let validated_api_base = Self::validate_api_base(requested_api_base, allow_insecure);
        let has_invalid_custom_api_base =
            !config.api_base.trim().is_empty() && validated_api_base.is_none();

        if has_invalid_custom_api_base {
            eprintln!(
                "Ignoring invalid Telegram api_base '{}' and disabling Telegram channel. \
Allowed default is '{}'. Set {}=1 only for local test endpoints.",
                config.api_base, DEFAULT_TELEGRAM_API_BASE, ALLOW_INSECURE_API_BASE_ENV
            );
        }

        Self {
            token: config.token.clone(),
            allow_from: config.allow_from.clone(),
            enabled: config.enabled && !config.token.is_empty() && !has_invalid_custom_api_base,
            api_base: validated_api_base.unwrap_or_else(|| DEFAULT_TELEGRAM_API_BASE.to_string()),
            client: reqwest::Client::new(),
        }
    }

    /// Create with a custom API base URL (for testing with mock servers).
    pub fn with_api_base(config: &TelegramConfig, api_base: &str) -> Self {
        Self {
            token: config.token.clone(),
            allow_from: config.allow_from.clone(),
            enabled: config.enabled && !config.token.is_empty(),
            api_base: api_base.to_string(),
            client: reqwest::Client::new(),
        }
    }

    /// Channel name for identification.
    pub fn name(&self) -> &str {
        "telegram"
    }

    /// Whether this channel is enabled and has a valid token.
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Check if a user is allowed to send messages.
    /// Returns true if the allow_from list is empty (all users allowed)
    /// or if the user_id is in the allow_from list.
    pub fn is_user_allowed(&self, user_id: &str) -> bool {
        self.allow_from.is_empty() || self.allow_from.iter().any(|id| id == user_id)
    }

    /// Parse a raw Telegram update into a TelegramMessage.
    /// Returns None if the update doesn't contain a text message.
    pub fn parse_update(update: &TelegramUpdate) -> Option<TelegramMessage> {
        let msg = update.message.as_ref()?;
        let text = msg.text.as_ref()?;
        let sender_id = msg
            .from
            .as_ref()
            .map(|u| u.id.to_string())
            .unwrap_or_default();
        let chat_id = msg.chat.id.to_string();

        Some(TelegramMessage {
            text: text.clone(),
            sender_id,
            chat_id,
        })
    }

    /// Parse a raw Telegram update as a voice message.
    /// Returns None if the update doesn't contain a voice attachment.
    pub fn parse_voice_update(
        update: &TelegramUpdate,
    ) -> Option<(String, String, String, Option<u64>)> {
        let msg = update.message.as_ref()?;
        let voice = msg.voice.as_ref()?;
        let sender_id = msg
            .from
            .as_ref()
            .map(|u| u.id.to_string())
            .unwrap_or_default();
        let chat_id = msg.chat.id.to_string();
        Some((sender_id, chat_id, voice.file_id.clone(), voice.file_size))
    }

    /// Get the file path for a Telegram file ID via the `getFile` API.
    pub async fn get_file(&self, file_id: &str) -> Result<String, DomainError> {
        let url = self.api_url("getFile");
        let body = serde_json::json!({ "file_id": file_id });

        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| DomainError::Channel(format!("Telegram getFile error: {}", e)))?;

        let status = response.status().as_u16();
        let response_text = response
            .text()
            .await
            .map_err(|e| DomainError::Channel(format!("failed to read getFile response: {}", e)))?;

        if status != 200 {
            return Err(DomainError::Channel(format!(
                "Telegram getFile failed ({}): {}",
                status, response_text
            )));
        }

        let parsed: serde_json::Value = serde_json::from_str(&response_text).map_err(|e| {
            DomainError::Channel(format!("failed to parse getFile response: {}", e))
        })?;

        parsed["result"]["file_path"]
            .as_str()
            .map(String::from)
            .ok_or_else(|| DomainError::Channel("getFile response missing file_path".to_string()))
    }

    /// Download a file from Telegram's file storage.
    pub async fn download_file(
        &self,
        file_path: &str,
        max_bytes: usize,
    ) -> Result<Vec<u8>, DomainError> {
        let url = format!("{}/file/bot{}/{}", self.api_base, self.token, file_path);

        let mut response =
            self.client.get(&url).send().await.map_err(|e| {
                DomainError::Channel(format!("Telegram file download error: {}", e))
            })?;

        let status = response.status().as_u16();
        if status != 200 {
            return Err(DomainError::Channel(format!(
                "Telegram file download failed ({})",
                status
            )));
        }

        if let Some(content_length) = response.content_length()
            && content_length > max_bytes as u64
        {
            return Err(DomainError::Channel(format!(
                "Telegram file exceeds size limit ({} > {} bytes)",
                content_length, max_bytes
            )));
        }

        let mut data = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| DomainError::Channel(format!("failed to read file bytes: {}", e)))?
        {
            if data.len() + chunk.len() > max_bytes {
                return Err(DomainError::Channel(format!(
                    "Telegram file exceeds size limit (>{} bytes)",
                    max_bytes
                )));
            }
            data.extend_from_slice(&chunk);
        }

        Ok(data)
    }

    /// Get the bot token.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Build the full URL for a Telegram Bot API method.
    fn api_url(&self, method: &str) -> String {
        format!("{}/bot{}/{}", self.api_base, self.token, method)
    }

    /// Send a text message to a chat.
    pub async fn send_message(&self, chat_id: &str, text: &str) -> Result<(), DomainError> {
        let url = self.api_url("sendMessage");
        let body = serde_json::json!({
            "chat_id": chat_id,
            "text": text,
        });

        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| DomainError::Channel(format!("Telegram HTTP error: {}", e)))?;

        let status = response.status().as_u16();
        if status != 200 {
            let body_text = response.text().await.unwrap_or_default();
            return Err(DomainError::Channel(format!(
                "Telegram sendMessage failed ({}): {}",
                status, body_text
            )));
        }

        Ok(())
    }

    /// Long-poll for updates from Telegram.
    /// Returns a list of updates with update_id > `offset`.
    pub async fn get_updates(
        &self,
        offset: i64,
        timeout: u32,
    ) -> Result<Vec<TelegramUpdate>, DomainError> {
        let url = self.api_url("getUpdates");
        let body = serde_json::json!({
            "offset": offset,
            "timeout": timeout,
            "allowed_updates": ["message"],
        });

        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| DomainError::Channel(format!("Telegram HTTP error: {}", e)))?;

        let status = response.status().as_u16();
        let response_text = response
            .text()
            .await
            .map_err(|e| DomainError::Channel(format!("failed to read response: {}", e)))?;

        if status != 200 {
            return Err(DomainError::Channel(format!(
                "Telegram getUpdates failed ({}): {}",
                status, response_text
            )));
        }

        let api_response: TelegramApiResponse<Vec<TelegramUpdate>> =
            serde_json::from_str(&response_text).map_err(|e| {
                DomainError::Channel(format!("failed to parse getUpdates response: {}", e))
            })?;

        if !api_response.ok {
            return Err(DomainError::Channel(format!(
                "Telegram API error: {}",
                api_response.description.unwrap_or_default()
            )));
        }

        Ok(api_response.result.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infrastructure::config::TelegramConfig;

    fn make_config(enabled: bool, token: &str, allow_from: Vec<&str>) -> TelegramConfig {
        TelegramConfig {
            enabled,
            token: token.to_string(),
            api_base: String::new(),
            allow_from: allow_from.into_iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn test_validate_api_base_accepts_default_https_host() {
        let base = TelegramChannel::validate_api_base("https://api.telegram.org", false);
        assert_eq!(base.as_deref(), Some("https://api.telegram.org"));
    }

    #[test]
    fn test_validate_api_base_rejects_http_without_override() {
        let base = TelegramChannel::validate_api_base("http://api.telegram.org", false);
        assert!(base.is_none());
    }

    #[test]
    fn test_validate_api_base_allows_http_with_override() {
        let base = TelegramChannel::validate_api_base("http://127.0.0.1:8080", true);
        assert_eq!(base.as_deref(), Some("http://127.0.0.1:8080"));
    }

    #[test]
    fn test_validate_api_base_rejects_non_telegram_host_without_override() {
        let base = TelegramChannel::validate_api_base("https://example.com", false);
        assert!(base.is_none());
    }

    #[test]
    fn test_validate_api_base_rejects_credentials_query_and_fragment() {
        assert!(
            TelegramChannel::validate_api_base("https://user:pass@api.telegram.org", false)
                .is_none()
        );
        assert!(
            TelegramChannel::validate_api_base("https://api.telegram.org?x=1", false).is_none()
        );
        assert!(
            TelegramChannel::validate_api_base("https://api.telegram.org#frag", false).is_none()
        );
    }

    #[test]
    fn test_invalid_custom_api_base_disables_channel() {
        let mut cfg = make_config(true, "123:ABC", vec![]);
        cfg.api_base = "http://example.com".to_string();
        let ch = TelegramChannel::new(&cfg);
        assert!(!ch.is_enabled());
        assert_eq!(ch.api_base, DEFAULT_TELEGRAM_API_BASE);
    }

    #[test]
    fn test_channel_name() {
        let ch = TelegramChannel::new(&make_config(true, "123:ABC", vec![]));
        assert_eq!(ch.name(), "telegram");
    }

    #[test]
    fn test_enabled_with_token() {
        let ch = TelegramChannel::new(&make_config(true, "123:ABC", vec![]));
        assert!(ch.is_enabled());
    }

    #[test]
    fn test_disabled_when_config_disabled() {
        let ch = TelegramChannel::new(&make_config(false, "123:ABC", vec![]));
        assert!(!ch.is_enabled());
    }

    #[test]
    fn test_disabled_when_no_token() {
        let ch = TelegramChannel::new(&make_config(true, "", vec![]));
        assert!(!ch.is_enabled());
    }

    #[test]
    fn test_allowed_user() {
        let ch = TelegramChannel::new(&make_config(true, "t", vec!["12345", "67890"]));
        assert!(ch.is_user_allowed("12345"));
        assert!(ch.is_user_allowed("67890"));
    }

    #[test]
    fn test_unauthorized_user_rejected() {
        let ch = TelegramChannel::new(&make_config(true, "t", vec!["12345"]));
        assert!(!ch.is_user_allowed("99999"));
    }

    #[test]
    fn test_empty_allow_from_allows_all() {
        let ch = TelegramChannel::new(&make_config(true, "t", vec![]));
        assert!(ch.is_user_allowed("99999"));
        assert!(ch.is_user_allowed("12345"));
    }

    #[test]
    fn test_parse_update_with_text() {
        let update = TelegramUpdate {
            update_id: 1,
            message: Some(TelegramUpdateMessage {
                message_id: 42,
                from: Some(TelegramUser {
                    id: 12345,
                    first_name: Some("John".to_string()),
                    username: Some("john".to_string()),
                }),
                chat: TelegramChat {
                    id: 12345,
                    chat_type: Some("private".to_string()),
                },
                text: Some("Hello agent".to_string()),
                voice: None,
            }),
        };

        let msg = TelegramChannel::parse_update(&update);
        assert!(msg.is_some());
        let msg = msg.unwrap();
        assert_eq!(msg.text, "Hello agent");
        assert_eq!(msg.sender_id, "12345");
        assert_eq!(msg.chat_id, "12345");
    }

    #[test]
    fn test_parse_update_no_message() {
        let update = TelegramUpdate {
            update_id: 1,
            message: None,
        };
        assert!(TelegramChannel::parse_update(&update).is_none());
    }

    #[test]
    fn test_parse_update_no_text() {
        let update = TelegramUpdate {
            update_id: 1,
            message: Some(TelegramUpdateMessage {
                message_id: 42,
                from: None,
                chat: TelegramChat {
                    id: 100,
                    chat_type: None,
                },
                text: None,
                voice: None,
            }),
        };
        assert!(TelegramChannel::parse_update(&update).is_none());
    }

    #[test]
    fn test_parse_update_json() {
        let json = r#"{
            "update_id": 100,
            "message": {
                "message_id": 42,
                "from": {"id": 555, "first_name": "Alice"},
                "chat": {"id": 555, "type": "private"},
                "text": "Hello from JSON"
            }
        }"#;
        let update: TelegramUpdate = serde_json::from_str(json).unwrap();
        let msg = TelegramChannel::parse_update(&update).unwrap();
        assert_eq!(msg.text, "Hello from JSON");
        assert_eq!(msg.sender_id, "555");
    }

    #[tokio::test]
    async fn test_send_message_success() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendMessage"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": true, "result": {}})),
            )
            .mount(&server)
            .await;

        let ch =
            TelegramChannel::with_api_base(&make_config(true, "123:ABC", vec![]), &server.uri());
        let result = ch.send_message("12345", "Hello!").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_send_message_error() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/sendMessage"))
            .respond_with(ResponseTemplate::new(400).set_body_string("Bad Request"))
            .mount(&server)
            .await;

        let ch =
            TelegramChannel::with_api_base(&make_config(true, "123:ABC", vec![]), &server.uri());
        let result = ch.send_message("12345", "Hello!").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("400"));
    }

    #[tokio::test]
    async fn test_get_updates_success() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let response_body = serde_json::json!({
            "ok": true,
            "result": [{
                "update_id": 101,
                "message": {
                    "message_id": 1,
                    "from": {"id": 42, "first_name": "Bob"},
                    "chat": {"id": 42, "type": "private"},
                    "text": "Hello bot"
                }
            }]
        });
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/getUpdates"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
            .mount(&server)
            .await;

        let ch =
            TelegramChannel::with_api_base(&make_config(true, "123:ABC", vec![]), &server.uri());
        let updates = ch.get_updates(0, 1).await.unwrap();
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].update_id, 101);
        let msg = TelegramChannel::parse_update(&updates[0]).unwrap();
        assert_eq!(msg.text, "Hello bot");
    }

    #[tokio::test]
    async fn test_get_updates_empty() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/bot123:ABC/getUpdates"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": true, "result": []})),
            )
            .mount(&server)
            .await;

        let ch =
            TelegramChannel::with_api_base(&make_config(true, "123:ABC", vec![]), &server.uri());
        let updates = ch.get_updates(0, 1).await.unwrap();
        assert!(updates.is_empty());
    }
}
