//! Derived session-scope metadata catalogue port (#2001 D3).
//!
//! Catalogue data is rebuildable and discardable; transcripts remain the
//! recovery authority. Adapters validate/version entries and rebuild from
//! authoritative session metadata after absence or corruption.

use std::future::Future;
use std::pin::Pin;

use crate::application::sessions::dto::scope_listing::{ScopeListQuery, ScopedSessionRow};
use crate::domain::error::DomainError;
use crate::domain::session_identity::SessionIdentity;

pub type CatalogueRows<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<ScopedSessionRow>, DomainError>> + Send + 'a>>;
pub type CatalogueUnit<'a> =
    Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + 'a>>;

/// Diagnostic when a rebuild skipped or repaired a bad record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogueRebuildReport {
    pub rows_written: usize,
    pub rows_skipped: usize,
    pub rebuilt_from_scratch: bool,
    pub notes: Vec<String>,
}

/// Port: derived, rebuildable metadata catalogue for scope-aware discovery.
pub trait SessionScopeCatalogue: Send + Sync {
    /// List or search rows per [`ScopeListQuery`].
    fn list(&self, query: &ScopeListQuery) -> CatalogueRows<'_>;

    /// Replace the catalogue atomically from authoritative rows.
    fn rebuild(&self, rows: Vec<ScopedSessionRow>) -> CatalogueUnit<'_>;

    /// Upsert one row after explicit association / save.
    fn upsert(&self, row: ScopedSessionRow) -> CatalogueUnit<'_>;

    /// Remove a row when a session is deleted (optional for MVP).
    fn remove(&self, identity: &SessionIdentity) -> CatalogueUnit<'_> {
        let _ = identity;
        Box::pin(async { Ok(()) })
    }

    /// Last rebuild report when the adapter retains one (default empty).
    fn last_rebuild_report(&self) -> Option<CatalogueRebuildReport> {
        None
    }
}

#[cfg(test)]
#[path = "scope_catalogue_tests.rs"]
mod tests;
