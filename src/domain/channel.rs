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
        ChannelTarget::TelegramChat(_) => Ok(()),
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
    fn test_validate_deliver_to_rejects_unknown_scheme() {
        let result = validate_deliver_to("slack:general");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("telegram:<chat_id>"));
    }
}
