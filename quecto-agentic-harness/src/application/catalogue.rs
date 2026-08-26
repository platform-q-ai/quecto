//! Application-owned provider/model catalogue use cases and ports.
//!
//! The application layer is the authority for resolving external catalogue
//! inputs into an immutable effective snapshot. It defines the ports it needs;
//! infrastructure implements those ports and interfaces only map requests and
//! responses to these use cases.

use std::sync::{Arc, RwLock};

use crate::domain::catalogue::{
    Availability, CatalogueSnapshot, ModelDescriptor, ModelRef, TransportKind, UnavailableReason,
};

/// One ordered catalogue input. Infrastructure implements this port for the
/// built-in layer, user-owned configuration, discovered metadata, and runtime
/// composition; the application owns the precedence between them.
pub trait CatalogueSource {
    /// Stable identifier used to report which layer failed to load.
    fn id(&self) -> &str;
    fn load(&self) -> Result<Vec<ModelDescriptor>, String>;
}

/// A source layer that could not be loaded. The layer is skipped rather than
/// failing resolution so one malformed input cannot erase the other layers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogueSourceError {
    pub source: String,
    pub error: String,
}

/// The effective catalogue plus the layers that were skipped while resolving it.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedCatalogue {
    pub snapshot: CatalogueSnapshot,
    pub skipped: Vec<CatalogueSourceError>,
}

/// Resolve ordered source layers into one immutable effective catalogue.
/// Later layers override earlier layers by stable provider/model identity.
#[derive(Debug, Default, Clone, Copy)]
pub struct ResolveCatalogueUseCase;

impl ResolveCatalogueUseCase {
    pub fn resolve(
        &self,
        generation: u64,
        layers: impl IntoIterator<Item = Vec<ModelDescriptor>>,
    ) -> CatalogueSnapshot {
        CatalogueSnapshot::merge_layers(generation, layers)
    }

    /// Resolve source layers in precedence order: earlier sources are the base,
    /// later sources upsert by stable provider/model identity. A source that
    /// fails to load is reported and skipped, so the remaining layers still
    /// publish a coherent catalogue.
    pub fn resolve_sources(
        &self,
        generation: u64,
        sources: &[&dyn CatalogueSource],
    ) -> ResolvedCatalogue {
        let mut layers = Vec::with_capacity(sources.len());
        let mut skipped = Vec::new();
        for source in sources {
            match source.load() {
                Ok(models) => layers.push(models),
                Err(error) => skipped.push(CatalogueSourceError {
                    source: source.id().to_string(),
                    error,
                }),
            }
        }
        ResolvedCatalogue {
            snapshot: self.resolve(generation, layers),
            skipped,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogueQuery {
    All,
    Runnable,
    Available,
    Configured,
    Known,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionFailure {
    UnknownModel,
    /// The bare model name matches more than one runnable provider, so the
    /// caller must qualify it.
    AmbiguousModel {
        candidates: Vec<String>,
    },
    Unavailable {
        reasons: Vec<UnavailableReason>,
    },
}

/// What a model reference resolved to in the effective catalogue.
#[derive(Debug, Clone, PartialEq)]
pub enum ModelSelection {
    /// A catalogue entry with known capabilities.
    Known(ModelDescriptor),
    /// A provider that routes any model id under its prefix, so the catalogue
    /// carries no descriptor and no capability limits are known.
    OpenRoute(ModelRef),
}

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

    pub fn current(&self) -> CatalogueSnapshot {
        // A panic elsewhere must not turn every later catalogue read into a
        // process-killing panic (#1128): the snapshot behind a poisoned lock is
        // still a valid published generation.
        self.current
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn publish(&self, snapshot: CatalogueSnapshot) {
        *self
            .current
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = snapshot;
    }
}

pub struct QueryCatalogueUseCase {
    store: CatalogueSnapshotStore,
}

impl QueryCatalogueUseCase {
    pub fn new(store: CatalogueSnapshotStore) -> Self {
        Self { store }
    }

    pub fn query(&self, filter: CatalogueQuery) -> CatalogueSnapshot {
        let snapshot = self.store.current();
        let models = snapshot
            .models()
            .iter()
            .filter(|model| match filter {
                // Derived views over one snapshot, narrowing in order: every
                // known entry, those with a usable configuration, those whose
                // transport also has an adapter, and finally those that can run
                // right now.
                CatalogueQuery::All | CatalogueQuery::Known => true,
                CatalogueQuery::Configured => model.configured,
                CatalogueQuery::Available => model.configured && model.adapter_supported(),
                CatalogueQuery::Runnable => model.availability.runnable(),
            })
            .cloned()
            .collect();
        // A projection narrows the model list, not the runtime's routing: the
        // open providers stay attached so consumers can still ask about them.
        CatalogueSnapshot::new(snapshot.generation, models)
            .with_open_providers(snapshot.open_providers().to_vec())
    }
}

pub struct ResolveModelSelectionUseCase {
    store: CatalogueSnapshotStore,
}

impl ResolveModelSelectionUseCase {
    pub fn new(store: CatalogueSnapshotStore) -> Self {
        Self { store }
    }

    pub fn resolve(&self, reference: &ModelRef) -> Result<ModelSelection, SelectionFailure> {
        let snapshot = self.store.current();
        let Some(model) = snapshot.find(reference) else {
            // A configured endpoint routes ids the catalogue cannot enumerate;
            // rejecting them would make explicitly configured providers
            // unusable.
            return if snapshot.accepts_any_model(reference.provider()) {
                Ok(ModelSelection::OpenRoute(reference.clone()))
            } else {
                Err(SelectionFailure::UnknownModel)
            };
        };
        if !model.availability.runnable() {
            // Availability is derived per catalogue entry, but the composed
            // runtime is the authority on what it can route: a prefix served by
            // a constructed provider stays selectable, keeping the entry's
            // metadata (and therefore its limits).
            if !snapshot.accepts_any_model(reference.provider()) {
                return Err(SelectionFailure::Unavailable {
                    reasons: model.availability.reasons().to_vec(),
                });
            }
        }
        Ok(ModelSelection::Known(model.clone()))
    }
}

/// The per-model limits an active model string resolves to in one snapshot:
/// `(output cap, context window)`, each `None` when not explicitly declared or
/// when the model does not resolve to a runnable entry. Startup, REPL, model
/// switching and reload all read limits through this one rule so a session
/// cannot silently gain or lose a clamp.
pub fn model_limits_in(snapshot: &CatalogueSnapshot, model: &str) -> (Option<u32>, Option<usize>) {
    let Ok(reference) = resolve_model_reference(snapshot, model) else {
        return (None, None);
    };
    let Some(descriptor) = snapshot.find(&reference) else {
        return (None, None);
    };
    // Declared limits are metadata, not a runtime capability: a model that
    // cannot run right now must not silently drop the clamp for a session that
    // is still using it.
    (
        descriptor
            .capabilities
            .max_tokens_explicit
            .then_some(descriptor.capabilities.max_tokens),
        descriptor
            .capabilities
            .context_window_explicit
            .then_some(descriptor.capabilities.context_window as usize),
    )
}

/// Resolve a user-supplied model string against one snapshot: a qualified
/// `provider/model` reference is taken as written, while a bare name resolves
/// only when exactly one runnable catalogue entry carries it.
pub fn resolve_model_reference(
    snapshot: &CatalogueSnapshot,
    model: &str,
) -> Result<ModelRef, SelectionFailure> {
    if let Ok(reference) = ModelRef::parse_qualified(model) {
        return Ok(reference);
    }
    let mut matches = snapshot
        .models()
        .iter()
        .filter(|descriptor| {
            descriptor.reference.model().as_str() == model && descriptor.availability.runnable()
        })
        .map(|descriptor| descriptor.reference.clone());
    let Some(unique) = matches.next() else {
        return Err(SelectionFailure::UnknownModel);
    };
    let rest: Vec<_> = matches.collect();
    if rest.is_empty() {
        return Ok(unique);
    }
    let mut candidates = vec![unique.qualified_id()];
    candidates.extend(rest.iter().map(ModelRef::qualified_id));
    Err(SelectionFailure::AmbiguousModel { candidates })
}

/// Application helper for translating adapter support into derived availability.
/// Catalogue data alone never makes a model runnable when no transport adapter
/// exists or credentials are unavailable.
pub fn derive_availability(
    transport: TransportKind,
    adapter_supported: bool,
    credential_available: bool,
) -> Availability {
    let mut reasons = Vec::new();
    if !adapter_supported {
        reasons.push(UnavailableReason::UnsupportedTransport { transport });
    }
    if !credential_available {
        reasons.push(UnavailableReason::MissingCredential);
    }
    if reasons.is_empty() {
        Availability::Runnable
    } else {
        Availability::KnownButUnavailable { reasons }
    }
}

#[cfg(test)]
#[path = "catalogue_tests.rs"]
mod tests;
