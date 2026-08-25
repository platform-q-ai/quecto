//! Application-owned catalogue refresh boundary.

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
    fn refresh_all_sources(&self) -> Vec<CatalogueRefreshOutcome>;
}

/// Production orchestration boundary shared by CLI and UDS interfaces.
pub struct CatalogueRefreshApplication<P> {
    port: P,
}

impl<P: CatalogueRefreshPort> CatalogueRefreshApplication<P> {
    pub fn new(port: P) -> Self {
        Self { port }
    }

    pub fn refresh(&self, source: &str) -> CatalogueRefreshOutcome {
        self.port.refresh_source(source)
    }

    pub fn refresh_all(&self) -> Vec<CatalogueRefreshOutcome> {
        self.port.refresh_all_sources()
    }
}

#[cfg(test)]
#[path = "catalogue_refresh_tests.rs"]
mod tests;
