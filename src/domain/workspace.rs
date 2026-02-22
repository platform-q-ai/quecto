use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use super::error::DomainError;

/// Port: read HEARTBEAT.md content from a workspace source.
pub trait HeartbeatTaskSource: Send + Sync {
    /// Returns Some(content) when HEARTBEAT.md exists, None otherwise.
    fn read_heartbeat_md(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Option<String>, DomainError>> + Send + '_>>;
}

/// Port: onboarding filesystem operations.
pub trait OnboardStore: Send + Sync {
    fn config_path(&self) -> PathBuf;
    fn workspace_path(&self) -> PathBuf;
    fn config_exists(&self) -> Result<bool, DomainError>;
    fn initialize(&self, templates: &[(&str, &str)]) -> Result<(), DomainError>;
}
