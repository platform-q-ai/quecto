//! Shared observations, not a transaction owner. Call admission under the store claim.
use crate::application::sessions::dto::resume_saved_session::ResumeDisposition;
use crate::application::sessions::ports::session_home::{SessionHomeCatalogue, WorkspaceDiscovery};
use crate::domain::{
    error::DomainError,
    session_home::{HomeAdmission, SessionHome, SessionHomeScope},
    session_identity::SessionIdentity,
};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
pub struct SessionHomeContext {
    pub catalogue: Arc<dyn SessionHomeCatalogue>,
    pub discovery: Arc<dyn WorkspaceDiscovery>,
    /// The execution directory, or why the process has none (an unreadable
    /// cwd is an observable unavailable, never an empty path).
    pub execution_dir: Result<PathBuf, String>,
}

impl SessionHomeContext {
    pub fn at(
        catalogue: Arc<dyn SessionHomeCatalogue>,
        discovery: Arc<dyn WorkspaceDiscovery>,
        execution_dir: PathBuf,
    ) -> Self {
        Self {
            catalogue,
            discovery,
            execution_dir: Ok(execution_dir),
        }
    }

    /// A fresh discovery of the current execution directory.
    pub async fn current(&self) -> Result<SessionHome, DomainError> {
        match &self.execution_dir {
            Ok(dir) => self.discovery.discover_async(dir).await,
            Err(reason) => Err(DomainError::Session(format!(
                "execution directory unavailable: {reason}"
            ))),
        }
    }

    /// The one admission decision: fresh current facts, the saved authority
    /// re-observed at its own directory, and the domain rule over both.
    pub async fn admit(&self, scope: &SessionHomeScope) -> Result<(), ResumeDisposition> {
        let current = self
            .current()
            .await
            .map_err(|error| ResumeDisposition::Unavailable(error.to_string()))?;
        let saved = match scope {
            SessionHomeScope::Scoped(saved) => saved,
            SessionHomeScope::LegacyUnscoped => return Err(ResumeDisposition::LegacyUnscoped),
            SessionHomeScope::Unavailable(reason) => {
                return Err(ResumeDisposition::Unavailable(reason.clone()));
            }
        };
        let observed = self
            .discovery
            .discover_async(&saved.execution_dir)
            .await
            .map_err(|error| ResumeDisposition::Unavailable(error.to_string()))?;
        match SessionHome::admission(saved, &observed, &current) {
            HomeAdmission::Eligible => Ok(()),
            HomeAdmission::HomeChanged => Err(ResumeDisposition::HomeChanged),
            HomeAdmission::DifferentExecutionDirectory => {
                Err(ResumeDisposition::DifferentExecutionDirectory)
            }
        }
    }

    pub async fn eligible(&self, scope: &SessionHomeScope) -> bool {
        self.admit(scope).await.is_ok()
    }

    pub async fn record_new(&self, identity: &SessionIdentity) -> Result<(), DomainError> {
        self.catalogue.record_new(identity, &self.current().await?)
    }
}
