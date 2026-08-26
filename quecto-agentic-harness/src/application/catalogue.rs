//! Application catalogue use cases and ports (epic #1193, slice 2).
//!
//! RED-phase skeleton: the API surface consumers and tests compile against.
//! Behaviour is implemented in the GREEN phase; every use case currently
//! signals `unimplemented`.

use std::sync::{Arc, RwLock};

use crate::domain::catalogue::{
    Availability, CatalogueEntry, CatalogueSnapshot, ProviderDescriptor, RejectedEntry,
    SourceLayer, TransportKind,
};

/// One named, ordered catalogue input. Infrastructure implements this port;
/// the application owns precedence between layers.
pub trait CatalogueSource: Send + Sync {
    /// Stable identifier used in structured per-source errors.
    fn id(&self) -> &str;
    /// The domain precedence layer this source feeds.
    fn layer(&self) -> SourceLayer;
    /// Load this source's entries. Parsing stays in infrastructure.
    fn load(&self) -> Result<Vec<CatalogueEntry>, String>;
}

/// Auth availability without exposing secret values.
pub trait CredentialStatusPort: Send + Sync {
    fn credential_available(&self, provider: &ProviderDescriptor) -> bool;
}

/// A source layer that could not be loaded; the layer is skipped so one
/// malformed input cannot erase the other layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogueSourceError {
    pub source: String,
    pub error: String,
}

/// Outcome of one resolve run: the snapshot that is now current, entries the
/// domain rejected, and sources that failed to load.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedCatalogue {
    pub snapshot: CatalogueSnapshot,
    pub rejected: Vec<RejectedEntry>,
    pub source_errors: Vec<CatalogueSourceError>,
}

/// Atomic holder of the current published snapshot generation.
#[derive(Debug, Clone)]
pub struct CatalogueSnapshotStore {
    current: Arc<RwLock<CatalogueSnapshot>>,
}

impl CatalogueSnapshotStore {
    pub fn new(initial: CatalogueSnapshot) -> Self {
        Self {
            current: Arc::new(RwLock::new(initial)),
        }
    }

    pub fn empty() -> Self {
        Self::new(CatalogueSnapshot::empty(0))
    }

    pub fn current(&self) -> CatalogueSnapshot {
        let _held = &self.current;
        unimplemented!("issue #1572: snapshot store read")
    }

    pub fn publish(&self, _snapshot: CatalogueSnapshot) {
        unimplemented!("issue #1572: atomic snapshot publication")
    }
}

/// Resolve effective catalogue: load all sources, validate via domain rules,
/// derive availability from credential status, publish one immutable
/// generation. When every source fails, the last valid snapshot is retained.
#[derive(Debug, Default, Clone, Copy)]
pub struct ResolveCatalogueUseCase;

impl ResolveCatalogueUseCase {
    pub fn resolve_and_publish(
        &self,
        _sources: &[&dyn CatalogueSource],
        _credentials: &dyn CredentialStatusPort,
        _store: &CatalogueSnapshotStore,
    ) -> ResolvedCatalogue {
        unimplemented!("issue #1572: resolve effective catalogue")
    }
}

/// Derived views over one snapshot, narrowing in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogueQuery {
    All,
    Known,
    Available,
    Runnable,
}

/// Query the current snapshot only — never re-reads configuration.
pub struct QueryCatalogueUseCase {
    store: CatalogueSnapshotStore,
}

impl QueryCatalogueUseCase {
    pub fn new(store: CatalogueSnapshotStore) -> Self {
        Self { store }
    }

    pub fn query(&self, _filter: CatalogueQuery) -> CatalogueSnapshot {
        let _held = &self.store;
        unimplemented!("issue #1572: query catalogue")
    }
}

/// Translate adapter support and credential status into derived availability.
pub fn derive_availability(
    _transport: TransportKind,
    _adapter_supported: bool,
    _credential_available: bool,
) -> Availability {
    unimplemented!("issue #1572: derive availability")
}

/// One row of the shared model-listing projection every read surface (CLI
/// listing, UDS queries, TUI model list) renders from.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelListingRow {
    pub qualified_id: String,
    pub display_name: Option<String>,
    pub runnable: bool,
}

/// The listing every consumer surface shows, tagged with the snapshot
/// generation it was projected from.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelListing {
    pub generation: u64,
    pub rows: Vec<ModelListingRow>,
}

/// Project the shared model listing from one snapshot.
pub fn project_model_listing(_snapshot: &CatalogueSnapshot) -> ModelListing {
    unimplemented!("issue #1572: shared model listing projection")
}

#[cfg(test)]
#[path = "catalogue_tests.rs"]
mod tests;
