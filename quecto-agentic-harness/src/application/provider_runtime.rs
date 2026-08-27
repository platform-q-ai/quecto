//! Application-owned provider runtime composition and model selection
//! (epic #1193, slice 3).
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
    CatalogueSnapshotStore, CatalogueSource, CredentialStatusPort, ResolveCatalogueUseCase,
    ResolvedCatalogue,
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
        // A panic elsewhere must not poison every later runtime read: the
        // snapshot behind a poisoned lock is still a fully published
        // generation (`publish` replaces it wholesale).
        self.current
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Atomically replace the published runtime snapshot. Readers see either
    /// the old or the new generation, never a partial state.
    fn publish(&self, snapshot: Arc<CatalogueRuntimeSnapshot>) {
        *self
            .current
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(snapshot);
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
        factory: &F,
        config: &C,
        runtime_inputs: &R,
        ports: &CompositionPorts<'_>,
    ) -> Result<ComposedRuntime, RuntimeCompositionError> {
        // The factory runs before anything is published: a failed composition
        // must leave both stores exactly as they were (the previous valid
        // runtime and catalogue generation stay current — or nothing, before
        // the first success).
        let provider = match factory.compose_runtime(config, runtime_inputs) {
            Ok(provider) => provider,
            Err(error) => {
                return Err(RuntimeCompositionError {
                    error,
                    retained: ports.runtime_store.current(),
                });
            }
        };
        let resolution = ResolveCatalogueUseCase.resolve_and_publish(
            ports.sources,
            ports.credentials,
            ports.catalogue_store,
        );
        let snapshot = Arc::new(CatalogueRuntimeSnapshot {
            catalogue: resolution.snapshot.clone(),
            provider,
        });
        // The runtime store is the coherent aggregate consumers read: its one
        // atomic publish pairs the routing runtime with the exact catalogue
        // generation it was composed from.
        ports.runtime_store.publish(snapshot.clone());
        Ok(ComposedRuntime {
            snapshot,
            resolution,
        })
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
        store: &RuntimeSnapshotStore,
        reference: &ModelRef,
    ) -> Result<ModelSelection, SelectionError> {
        let Some(snapshot) = store.current() else {
            return Err(SelectionError::NoRuntime);
        };
        select_in_snapshot(&snapshot, reference)
    }
}

/// Resolve a model reference against one published runtime generation:
/// descriptor plus runnable provider, or the structured reason the catalogue
/// derived for it. The catalogue entry — identity, transport, auth path, and
/// metadata — is returned exactly as the catalogue shows it; selection never
/// substitutes a different auth identity.
pub fn select_in_snapshot(
    snapshot: &CatalogueRuntimeSnapshot,
    reference: &ModelRef,
) -> Result<ModelSelection, SelectionError> {
    let Some(entry) = snapshot.catalogue.find(reference) else {
        return Err(SelectionError::UnknownModel {
            reference: reference.qualified_id(),
        });
    };
    if !entry.model.availability.is_runnable() {
        return Err(SelectionError::NotRunnable {
            reference: reference.clone(),
            reasons: entry.model.availability.reasons().to_vec(),
        });
    }
    Ok(ModelSelection {
        entry: entry.clone(),
        provider: snapshot.provider.clone(),
        generation: snapshot.generation(),
    })
}

#[cfg(test)]
#[path = "provider_runtime_tests.rs"]
mod tests;
