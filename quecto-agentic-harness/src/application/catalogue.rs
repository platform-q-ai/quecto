//! Application catalogue use cases and ports (epic #1193, slice 2).
//!
//! The application layer is the authority for resolving catalogue sources into
//! one published, immutable snapshot generation. It defines the ports it
//! needs; infrastructure implements them, and every read surface (CLI listing,
//! UDS queries, TUI model list) consumes query results projected from the
//! current snapshot.

use std::sync::{Arc, RwLock};

use crate::domain::catalogue::{
    Availability, AvailabilityStatus, CatalogueEntry, CatalogueSnapshot, ProviderDescriptor,
    RejectedEntry, SourceLayer, TransportKind, UnavailableReason, resolve_catalogue,
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
        // A panic elsewhere must not turn every later catalogue read into a
        // process-killing panic: the snapshot behind a poisoned lock is still
        // a fully published generation (`publish` replaces it wholesale).
        self.current
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Atomically replace the current snapshot. Readers see either the old or
    /// the new generation, never a partial state.
    pub fn publish(&self, snapshot: CatalogueSnapshot) {
        *self
            .current
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = snapshot;
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
        sources: &[&dyn CatalogueSource],
        credentials: &dyn CredentialStatusPort,
        store: &CatalogueSnapshotStore,
    ) -> ResolvedCatalogue {
        let mut layers = Vec::with_capacity(sources.len());
        let mut source_errors = Vec::new();
        for source in sources {
            match source.load() {
                Ok(entries) => layers.push((source.layer(), entries)),
                Err(error) => source_errors.push(CatalogueSourceError {
                    source: source.id().to_string(),
                    error,
                }),
            }
        }
        if layers.is_empty() && !source_errors.is_empty() {
            // Every source failed: the last valid snapshot stays published so
            // consumers keep a coherent catalogue instead of an empty one.
            return ResolvedCatalogue {
                snapshot: store.current(),
                rejected: Vec::new(),
                source_errors,
            };
        }
        for (_, entries) in &mut layers {
            for entry in entries {
                entry.model.availability = derive_availability(
                    entry.provider.transport,
                    transport_has_adapter(entry.provider.transport),
                    credentials.credential_available(&entry.provider),
                );
            }
        }
        let generation = store.current().generation() + 1;
        let resolution = resolve_catalogue(generation, layers);
        store.publish(resolution.snapshot.clone());
        ResolvedCatalogue {
            snapshot: resolution.snapshot,
            rejected: resolution.rejected,
            source_errors,
        }
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

    /// Project a filtered view of the current snapshot. Reads the snapshot
    /// store only — this use case holds no source ports, so it cannot re-read
    /// configuration by construction.
    pub fn query(&self, filter: CatalogueQuery) -> CatalogueSnapshot {
        let snapshot = self.store.current();
        snapshot.filtered(|entry| match filter {
            // Derived views narrowing in order: everything the catalogue
            // knows, entries whose status reached at least Available (adapter
            // present, only a credential possibly missing), and entries that
            // can run right now.
            CatalogueQuery::All | CatalogueQuery::Known => true,
            CatalogueQuery::Available => {
                entry.model.availability.status() >= AvailabilityStatus::Available
            }
            CatalogueQuery::Runnable => entry.model.availability.is_runnable(),
        })
    }
}

/// Whether a transport adapter exists for this transport kind. Every kind the
/// domain enumerates has an adapter today; the parameter stays explicit on
/// [`derive_availability`] so a future transport without one derives honestly.
fn transport_has_adapter(_transport: TransportKind) -> bool {
    true
}

/// Translate adapter support and credential status into derived availability.
pub fn derive_availability(
    transport: TransportKind,
    adapter_supported: bool,
    credential_available: bool,
) -> Availability {
    if adapter_supported && credential_available {
        return Availability::runnable();
    }
    let mut reasons = Vec::new();
    if !credential_available {
        reasons.push(UnavailableReason::MissingCredential);
    }
    if !adapter_supported {
        reasons.push(UnavailableReason::UnsupportedTransport { transport });
    }
    // No adapter caps the ladder at Configured; a missing credential alone
    // leaves the entry Available (everything but the credential is in place).
    let status = if adapter_supported {
        AvailabilityStatus::Available
    } else {
        AvailabilityStatus::Configured
    };
    Availability::unavailable(status, reasons)
        .expect("non-runnable status with at least one reason is always constructible")
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
pub fn project_model_listing(snapshot: &CatalogueSnapshot) -> ModelListing {
    ModelListing {
        generation: snapshot.generation(),
        rows: snapshot
            .entries()
            .iter()
            .map(|entry| ModelListingRow {
                qualified_id: entry.reference().qualified_id(),
                display_name: entry.model.display_name.clone(),
                runnable: entry.model.availability.is_runnable(),
            })
            .collect(),
    }
}

/// The per-model limits a qualified `provider/model` reference declares in one
/// snapshot: `(output cap, context window)`, each `None` when not explicitly
/// declared. Every limits read goes through this one rule so a session cannot
/// silently gain or lose a clamp.
pub fn model_limits_in(
    snapshot: &CatalogueSnapshot,
    qualified: &str,
) -> (Option<u32>, Option<usize>) {
    let Ok(reference) = crate::domain::catalogue::ModelRef::parse_qualified(qualified) else {
        return (None, None);
    };
    let Some(entry) = snapshot.find(&reference) else {
        return (None, None);
    };
    // A synthesized default is not a real limit and must not clamp: only
    // explicitly declared values count.
    let capabilities = &entry.model.capabilities;
    (
        capabilities
            .max_output_tokens_explicit
            .then_some(capabilities.max_output_tokens),
        capabilities
            .context_window_explicit
            .then_some(capabilities.context_window as usize),
    )
}

#[cfg(test)]
#[path = "catalogue_tests.rs"]
mod tests;
