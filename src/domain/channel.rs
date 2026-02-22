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
