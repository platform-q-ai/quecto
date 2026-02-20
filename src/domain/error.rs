use thiserror::Error;

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("provider error: {0}")]
    Provider(String),

    #[error("tool error: {0}")]
    Tool(String),

    #[error("session error: {0}")]
    Session(String),

    #[error("channel error: {0}")]
    Channel(String),

    #[error("security violation: {0}")]
    Security(String),

    #[error("config error: {0}")]
    Config(String),

    #[error("{0}")]
    Other(String),
}
