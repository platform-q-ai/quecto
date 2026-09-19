//! Home authority, derived discovery index, and canonical workspace facts.
use crate::domain::{
    error::DomainError,
    session::SessionSummary,
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

/// One saved session as the metadata query sees it (#2010): the listing
/// summary (key, title, count, time) and the home the catalogue lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMetadataRecord {
    pub summary: SessionSummary,
    pub home: SessionHomeScope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMetadataSnapshot {
    pub records: Vec<SessionMetadataRecord>,
    pub diagnostics: Vec<String>,
    pub rebuilt: bool,
}

/// A port answer that may isolate blocking work off the async executor.
pub type Answer<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, DomainError>> + Send + 'a>>;

pub trait WorkspaceDiscovery: Send + Sync {
    fn discover(&self, path: &Path) -> Result<SessionHome, DomainError>;
    /// Async discovery entry point; process-spawning adapters isolate blocking work.
    fn discover_async(&self, path: &Path) -> Answer<'_, SessionHome> {
        let home = self.discover(path);
        Box::pin(async move { home })
    }
}

pub trait SessionHomeCatalogue: Send + Sync {
    /// Exact authoritative lookup, independent of the derived catalogue.
    fn read(&self, identity: &SessionIdentity) -> Result<SessionHomeScope, DomainError>;
    fn list(&self) -> Result<HomeCatalogueSnapshot, DomainError>;
    /// Async discovery entry point; filesystem adapters isolate blocking work.
    fn list_async(&self) -> Answer<'_, HomeCatalogueSnapshot> {
        Box::pin(async { self.list() })
    }
    /// The metadata query (#2010): every listed session exactly once, newest
    /// first, each with its listing summary and the home the listing shows —
    /// the catalogue's row, or the exact authority for a record the strict
    /// catalogue rejected. Validated against authority like [`Self::list`]
    /// (same diagnostics, same recovery), and never a transcript read for a
    /// record whose stamp is unchanged. An adapter without the projection is
    /// observably unavailable, never an empty answer.
    fn metadata(&self) -> Answer<'_, SessionMetadataSnapshot> {
        Box::pin(async {
            Err(DomainError::Session(
                "session metadata query unavailable".into(),
            ))
        })
    }
    /// Only a new persistent identity can acquire a home. Call under ownership
    /// before the first transcript save; existing authority is immutable here.
    fn record_new(&self, identity: &SessionIdentity, home: &SessionHome)
    -> Result<(), DomainError>;
    /// A home without a transcript is no session (#2009): discard it under
    /// ownership so the key is not locked to a directory with no history.
    /// Authority beside a transcript is never touched.
    fn discard_orphan(&self, identity: &SessionIdentity) -> Result<(), DomainError>;
}
