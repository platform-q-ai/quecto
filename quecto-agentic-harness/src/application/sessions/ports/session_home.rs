//! Home authority, derived discovery index, and canonical workspace facts.
use crate::domain::{
    error::DomainError,
    session_home::{SessionHome, SessionHomeScope},
    session_identity::SessionIdentity,
};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HomeCatalogueSnapshot {
    pub entries: Vec<(SessionIdentity, SessionHomeScope)>,
    pub diagnostics: Vec<String>,
    pub rebuilt: bool,
}

pub trait WorkspaceDiscovery: Send + Sync {
    fn discover(&self, path: &Path) -> Result<SessionHome, DomainError>;
}

pub trait SessionHomeCatalogue: Send + Sync {
    /// Exact authoritative lookup, independent of the derived catalogue.
    fn read(&self, identity: &SessionIdentity) -> Result<SessionHomeScope, DomainError>;
    fn list(&self) -> Result<HomeCatalogueSnapshot, DomainError>;
    /// Only a new persistent identity can acquire a home. Call under ownership
    /// before the first transcript save; existing authority is immutable here.
    fn record_new(&self, identity: &SessionIdentity, home: &SessionHome)
    -> Result<(), DomainError>;
}
