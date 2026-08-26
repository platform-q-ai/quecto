//! Application-owned provider runtime composition and model selection
//! (epic #1193, slice 3).
//!
//! RED-phase skeleton: the API surface consumers and tests compile against.
//! Behaviour is implemented in the GREEN phase; every use case currently
//! signals `unimplemented`.
//!
//! The application owns the composition use-case seam: from the current
//! resolved catalogue generation it constructs the concrete provider runtime
//! (via a factory port implemented in infrastructure) and publishes runtime
//! plus catalogue as one coherent generation. Concrete configuration and
//! runtime input shapes stay outside this layer: callers select those types
//! via the generic factory implementation, keeping application independent of
//! infrastructure.

use std::sync::{Arc, RwLock};

use crate::application::catalogue::{
    CatalogueSnapshotStore, CatalogueSource, CredentialStatusPort, ResolvedCatalogue,
};
use crate::domain::catalogue::{CatalogueEntry, CatalogueSnapshot, ModelRef, UnavailableReason};
use crate::domain::provider::LlmProvider;

/// Port for constructing the concrete execution runtime (providers, OAuth
/// wrapping, retry decoration, router composition) from resolved provider
/// definitions. Infrastructure implements this; the application orchestrates.
pub trait ProviderRuntimeFactory<C, R>: Send + Sync {
    fn compose_runtime(
        &self,
        config: &C,
        runtime_inputs: &R,
    ) -> Result<Arc<dyn LlmProvider>, String>;
}

/// Provider runtime and catalogue published as one immutable generation.
/// Both halves always describe the same resolved generation.
#[derive(Debug, Clone)]
pub struct CatalogueRuntimeSnapshot {
    pub catalogue: Arc<CatalogueSnapshot>,
    pub provider: Arc<dyn LlmProvider>,
}

impl CatalogueRuntimeSnapshot {
    pub fn generation(&self) -> u64 {
        self.catalogue.generation()
    }
}

/// Atomic holder of the current published runtime generation. `None` until
/// the first successful composition.
#[derive(Debug, Clone, Default)]
pub struct RuntimeSnapshotStore {
    current: Arc<RwLock<Option<Arc<CatalogueRuntimeSnapshot>>>>,
}

impl RuntimeSnapshotStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// The current published runtime snapshot, or `None` before the first
    /// successful composition.
    pub fn current(&self) -> Option<Arc<CatalogueRuntimeSnapshot>> {
        let _held = &self.current;
        unimplemented!("issue #1573: runtime snapshot store read")
    }
}

/// A composition failure. The previously published runtime (if any) stays
/// current; it is echoed here so callers can report what is still in effect.
#[derive(Debug, Clone)]
pub struct RuntimeCompositionError {
    pub error: String,
    pub retained: Option<Arc<CatalogueRuntimeSnapshot>>,
}

/// Outcome of one successful compose-and-publish run.
#[derive(Debug, Clone)]
pub struct ComposedRuntime {
    pub snapshot: Arc<CatalogueRuntimeSnapshot>,
    pub resolution: ResolvedCatalogue,
}

/// The catalogue-side ports one composition run reads and publishes through:
/// the catalogue sources and credential status it resolves from, and the two
/// stores it publishes to as one generation.
pub struct CompositionPorts<'a> {
    pub sources: &'a [&'a dyn CatalogueSource],
    pub credentials: &'a dyn CredentialStatusPort,
    pub catalogue_store: &'a CatalogueSnapshotStore,
    pub runtime_store: &'a RuntimeSnapshotStore,
}

/// Compose the provider runtime from the same resolved generation as the
/// catalogue and publish both atomically: consumers can never observe a new
/// catalogue with stale routing or vice versa. A failed composition publishes
/// nothing — the previous valid runtime and catalogue generation stay current.
#[derive(Debug, Default, Clone, Copy)]
pub struct ComposeProviderRuntimeUseCase;

impl ComposeProviderRuntimeUseCase {
    pub fn new() -> Self {
        Self
    }

    pub fn compose_and_publish<C, R, F: ProviderRuntimeFactory<C, R>>(
        &self,
        _factory: &F,
        _config: &C,
        _runtime_inputs: &R,
        _ports: &CompositionPorts<'_>,
    ) -> Result<ComposedRuntime, RuntimeCompositionError> {
        unimplemented!("issue #1573: compose provider runtime use case")
    }
}

/// Why a model selection cannot produce a runnable provider.
#[derive(Debug, Clone, PartialEq)]
pub enum SelectionError {
    /// No runtime has been composed yet.
    NoRuntime,
    /// The catalogue generation does not know this model reference.
    UnknownModel { reference: String },
    /// The model is known but not runnable, for these structured reasons.
    NotRunnable {
        reference: ModelRef,
        reasons: Vec<UnavailableReason>,
    },
}

/// A successful selection: the catalogue entry (descriptor, identity,
/// metadata) and the runnable provider from the same generation.
#[derive(Debug, Clone)]
pub struct ModelSelection {
    pub entry: CatalogueEntry,
    pub provider: Arc<dyn LlmProvider>,
    pub generation: u64,
}

/// Resolve a stable model reference against the current published runtime
/// generation: descriptor plus runnable provider, or a structured reason.
#[derive(Debug, Default, Clone, Copy)]
pub struct ResolveModelSelectionUseCase;

impl ResolveModelSelectionUseCase {
    pub fn new() -> Self {
        Self
    }

    pub fn select(
        &self,
        _store: &RuntimeSnapshotStore,
        _reference: &ModelRef,
    ) -> Result<ModelSelection, SelectionError> {
        unimplemented!("issue #1573: resolve model selection use case")
    }
}

#[cfg(test)]
#[path = "provider_runtime_tests.rs"]
mod tests;
