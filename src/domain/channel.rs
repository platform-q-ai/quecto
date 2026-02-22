use std::future::Future;
use std::pin::Pin;

use super::error::DomainError;

/// Port: outbound message delivery channel.
pub trait Channel: Send + Sync {
    /// Human-readable channel name (for logging/diagnostics).
    fn name(&self) -> &str;

    /// Send a message to an implementation-defined target string.
    fn send_message<'a>(
        &'a self,
        target: &'a str,
        text: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + 'a>>;
}
