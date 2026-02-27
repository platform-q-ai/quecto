use std::future::Future;
use std::pin::Pin;

use super::error::DomainError;

/// Typed outbound target for channel delivery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelTarget {
    TelegramChat(String),
    Unsupported(String),
}

impl ChannelTarget {
    pub fn parse(raw: &str) -> Self {
        if let Some(chat_id) = raw.strip_prefix("telegram:") {
            return Self::TelegramChat(chat_id.to_string());
        }
        Self::Unsupported(raw.to_string())
    }
}

/// Validate whether a deliver_to string is a recognized channel target.
/// Returns `Ok(())` for valid targets, `Err(reason)` for invalid ones.
pub fn validate_deliver_to(raw: &str) -> Result<(), String> {
    match ChannelTarget::parse(raw) {
        ChannelTarget::TelegramChat(ref chat_id) => {
            if chat_id.is_empty() {
                return Err(
                    "invalid deliver_to 'telegram:': chat_id must not be empty. Example: 'telegram:12345'".to_string()
                );
            }
            // Telegram chat IDs are numeric (may be negative for groups).
            let id_str = chat_id.strip_prefix('-').unwrap_or(chat_id);
            if id_str.is_empty() || !id_str.chars().all(|c| c.is_ascii_digit()) {
                return Err(format!(
                    "invalid deliver_to 'telegram:{}': chat_id must be numeric. Example: 'telegram:12345'",
                    chat_id
                ));
            }
            Ok(())
        }
        ChannelTarget::Unsupported(s) => Err(format!(
            "invalid deliver_to '{}': must be 'telegram:<chat_id>'. Example: 'telegram:12345'",
            s
        )),
    }
}

/// Port: outbound message delivery channel.
pub trait Channel: Send + Sync {
    /// Human-readable channel name (for logging/diagnostics).
    fn name(&self) -> &str;

    /// Send a message to an implementation-defined target string.
    fn send_message<'a>(
        &'a self,
        target: &'a ChannelTarget,
        text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + 'a>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_deliver_to_valid_telegram() {
        assert!(validate_deliver_to("telegram:12345").is_ok());
    }

    #[test]
    fn test_validate_deliver_to_valid_negative_chat_id() {
        // Group chats have negative IDs
        assert!(validate_deliver_to("telegram:-100123456").is_ok());
    }

    #[test]
    fn test_validate_deliver_to_rejects_current() {
        let result = validate_deliver_to("current");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid deliver_to"));
    }

    #[test]
    fn test_validate_deliver_to_rejects_empty() {
        let result = validate_deliver_to("");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_deliver_to_rejects_empty_chat_id() {
        let result = validate_deliver_to("telegram:");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must not be empty"));
    }

    #[test]
    fn test_validate_deliver_to_rejects_non_numeric_chat_id() {
        let result = validate_deliver_to("telegram:abc");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must be numeric"));
    }

    #[test]
    fn test_validate_deliver_to_rejects_bare_minus() {
        let result = validate_deliver_to("telegram:-");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must be numeric"));
    }

    #[test]
    fn test_validate_deliver_to_rejects_unknown_scheme() {
        let result = validate_deliver_to("slack:general");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("telegram:<chat_id>"));
    }
}
