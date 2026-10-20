//! What inspecting the effective configuration reports (#1843).

use crate::application::configuration::dto::{ConfigSources, EffectiveConfigError};
use crate::application::configuration::ports::{
    RunSettings, StoredCredential, StoredCredentialsUnavailable,
};

/// The effective configuration of a run, as `quecto status` shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationInspection {
    /// Which files produced it.
    pub sources: ConfigSources,
    pub settings: RunSettings,
    /// The stored credentials, or what kind of failure kept them from
    /// being told: an unreadable credential store does not hide the rest
    /// of the configuration.
    pub stored_credentials: Result<Vec<StoredCredential>, StoredCredentialsUnavailable>,
}

/// Why the effective configuration cannot be inspected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InspectionError {
    /// A layer could not be read, checked or merged.
    Effective(EffectiveConfigError),
    /// The merged document does not realize, in the schema's words.
    Settings(String),
}

impl std::fmt::Display for InspectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InspectionError::Effective(error) => write!(f, "{error}"),
            InspectionError::Settings(reason) => f.write_str(reason),
        }
    }
}
