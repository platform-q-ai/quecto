//! Shared observations, not a transaction owner. Call admission under the store claim.
use crate::application::sessions::dto::resume_saved_session::ResumeDisposition;
use crate::application::sessions::ports::session_home::{SessionHomeCatalogue, WorkspaceDiscovery};
use crate::domain::resume_decision::ResumeDecisionKind;
use crate::domain::{
    error::DomainError,
    session_home::{HomeAdmission, SessionHome, SessionHomeScope},
    session_identity::SessionIdentity,
};
use std::{path::PathBuf, sync::Arc};

/// Why a saved home does not admit a restore in this runtime (#2011).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HomeObstacle {
    /// The user can decide: the kind, and why the home was not observable.
    Decision(ResumeDecisionKind, Option<String>),
    /// This runtime's own execution directory is undiscoverable: no decision
    /// made here could be validated against it.
    CurrentUnavailable(String),
}

impl HomeObstacle {
    fn decision(kind: ResumeDecisionKind) -> Self {
        Self::Decision(kind, None)
    }

    pub fn into_disposition(self) -> ResumeDisposition {
        use ResumeDecisionKind as Kind;
        match self {
            Self::Decision(Kind::LegacyUnscoped, _) => ResumeDisposition::LegacyUnscoped,
            Self::Decision(Kind::HomeChanged, _) => ResumeDisposition::HomeChanged,
            Self::Decision(Kind::CrossFolder, _) => ResumeDisposition::DifferentExecutionDirectory,
            Self::Decision(Kind::HomeMissing | Kind::HomeUnknown, detail) => {
                ResumeDisposition::Unavailable(detail.unwrap_or_default())
            }
            Self::CurrentUnavailable(reason) => ResumeDisposition::Unavailable(reason),
        }
    }
}

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
    /// re-observed at its own directory, and the domain rule over both. Every
    /// input is an affirmative observation: one that is missing, ambiguous or
    /// denied yields an obstacle, never an admission.
    pub async fn classify(&self, scope: &SessionHomeScope) -> Result<(), HomeObstacle> {
        let current = self
            .current()
            .await
            .map_err(|error| HomeObstacle::CurrentUnavailable(error.to_string()))?;
        let saved = match scope {
            SessionHomeScope::Scoped(saved) => saved,
            SessionHomeScope::LegacyUnscoped => {
                return Err(HomeObstacle::decision(ResumeDecisionKind::LegacyUnscoped));
            }
            SessionHomeScope::Unavailable(reason) => {
                return Err(HomeObstacle::Decision(
                    ResumeDecisionKind::HomeUnknown,
                    Some(reason.clone()),
                ));
            }
        };
        let observed = self
            .discovery
            .discover_async(&saved.execution_dir)
            .await
            .map_err(|error| {
                HomeObstacle::Decision(ResumeDecisionKind::HomeMissing, Some(error.to_string()))
            })?;
        match SessionHome::admission(saved, &observed, &current) {
            HomeAdmission::Eligible => Ok(()),
            HomeAdmission::HomeChanged => {
                Err(HomeObstacle::decision(ResumeDecisionKind::HomeChanged))
            }
            HomeAdmission::DifferentExecutionDirectory => {
                Err(HomeObstacle::decision(ResumeDecisionKind::CrossFolder))
            }
        }
    }

    /// [`Self::classify`] as the startup disposition (#2009).
    pub async fn admit(&self, scope: &SessionHomeScope) -> Result<(), ResumeDisposition> {
        self.classify(scope)
            .await
            .map_err(HomeObstacle::into_disposition)
    }

    pub async fn eligible(&self, scope: &SessionHomeScope) -> bool {
        self.admit(scope).await.is_ok()
    }

    pub async fn record_new(&self, identity: &SessionIdentity) -> Result<(), DomainError> {
        self.catalogue.record_new(identity, &self.current().await?)
    }
}
