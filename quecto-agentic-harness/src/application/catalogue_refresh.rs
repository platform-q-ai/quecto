//! Application-level catalogue refresh use case and result types.
//!
//! Infrastructure performs provider-specific HTTP and persistence; interfaces
//! call this use case instead of owning discovery/refresh semantics.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogueRefreshStatus {
    Refreshed { models: usize },
    Skipped { reason: String },
    Failed { error: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogueRefreshOutcome {
    pub source: String,
    pub status: CatalogueRefreshStatus,
}
