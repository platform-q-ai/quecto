use std::path::PathBuf;

use super::error::DomainError;

/// Port: onboarding filesystem operations.
pub trait OnboardStore: Send + Sync {
    fn config_path(&self) -> PathBuf;
    fn workspace_path(&self) -> PathBuf;
    fn config_exists(&self) -> Result<bool, DomainError>;
    fn initialize(&self) -> Result<(), DomainError>;
}
