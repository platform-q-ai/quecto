use super::error::DomainError;

/// Port: a chat channel that receives user messages and sends responses.
pub trait Channel: Send + Sync {
    /// Human-readable channel name (e.g. "telegram").
    fn name(&self) -> &str;

    /// Start listening for incoming messages.
    fn start(&mut self) -> impl std::future::Future<Output = Result<(), DomainError>> + Send;

    /// Gracefully stop the channel.
    fn stop(&mut self) -> impl std::future::Future<Output = Result<(), DomainError>> + Send;
}
