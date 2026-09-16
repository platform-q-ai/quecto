//! Shared observations, not a transaction owner. Call admission under the store claim.
use crate::application::sessions::ports::session_home::{SessionHomeCatalogue, WorkspaceDiscovery};
use crate::domain::{
    error::DomainError,
    session_home::{SessionHome, SessionHomeScope},
    session_identity::SessionIdentity,
};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
pub struct SessionHomeContext {
    pub catalogue: Arc<dyn SessionHomeCatalogue>,
    pub discovery: Arc<dyn WorkspaceDiscovery>,
    pub execution_dir: PathBuf,
}

impl SessionHomeContext {
    pub fn current(&self) -> Result<SessionHome, DomainError> {
        self.discovery.discover(&self.execution_dir)
    }

    pub fn eligible(&self, scope: &SessionHomeScope) -> bool {
        match (scope, self.current()) {
            (SessionHomeScope::Scoped(home), Ok(current)) => {
                matches!(self.discovery.discover(&home.execution_dir), Ok(observed)
                    if observed == *home && observed.execution_dir == current.execution_dir && observed.group == current.group)
            }
            _ => false,
        }
    }

    pub fn record_new(&self, identity: &SessionIdentity) -> Result<(), DomainError> {
        self.catalogue.record_new(identity, &self.current()?)
    }
}

#[cfg(test)]
#[path = "session_home_tests.rs"]
mod tests;
