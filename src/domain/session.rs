use std::future::Future;
use std::pin::Pin;

use super::{error::DomainError, message::Message};

/// A conversation session identified by a unique key.
#[derive(Debug, Clone)]
pub struct Session {
    /// Unique key, e.g. "telegram:12345" or "cli:default".
    pub key: String,
    /// Ordered conversation history.
    pub messages: Vec<Message>,
}

impl Session {
    /// Create a new empty session.
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            messages: vec![],
        }
    }

    /// Build a session key from channel and user ID.
    pub fn build_key(channel: &str, user_id: &str) -> String {
        format!("{}:{}", channel, user_id)
    }
}

/// Port: persistent storage for conversation sessions.
pub trait SessionStore: Send + Sync {
    /// Load a session by key. Returns None if no session exists.
    fn load(
        &self,
        key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Option<Session>, DomainError>> + Send + '_>>;

    /// Save (create or update) a session.
    fn save(
        &self,
        session: &Session,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>>;

    /// Check if a session exists.
    fn exists(
        &self,
        key: &str,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DomainError>> + Send + '_>>;
}
