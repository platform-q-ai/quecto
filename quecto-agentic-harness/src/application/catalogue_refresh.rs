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

pub trait CatalogueRefreshPort {
    fn refresh_source(&self, source: &str) -> CatalogueRefreshOutcome;
}

pub trait CatalogueRefreshAllPort: CatalogueRefreshPort {
    fn refresh_all_sources(&self) -> Vec<CatalogueRefreshOutcome>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct RefreshCatalogueSourceUseCase;

impl RefreshCatalogueSourceUseCase {
    pub fn new() -> Self {
        Self
    }

    pub fn refresh<P: CatalogueRefreshPort>(
        &self,
        port: &P,
        source: &str,
    ) -> CatalogueRefreshOutcome {
        port.refresh_source(source)
    }

    pub fn refresh_all<P: CatalogueRefreshAllPort>(
        &self,
        port: &P,
    ) -> Vec<CatalogueRefreshOutcome> {
        port.refresh_all_sources()
    }
}

#[cfg(test)]
#[path = "catalogue_refresh_tests.rs"]
mod tests;
