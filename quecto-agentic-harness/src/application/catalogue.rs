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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogueQuery {
    All,
    Runnable,
    Available,
    Known,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionFailure {
    UnknownModel,
    Unavailable { reasons: Vec<UnavailableReason> },
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
        self.current
            .read()
            .expect("catalogue snapshot lock poisoned")
            .clone()
    }

    pub fn publish(&self, snapshot: CatalogueSnapshot) {
        *self
            .current
            .write()
            .expect("catalogue snapshot lock poisoned") = snapshot;
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
                CatalogueQuery::All | CatalogueQuery::Known => true,
                CatalogueQuery::Available | CatalogueQuery::Runnable => {
                    model.availability.runnable()
                }
            })
            .cloned()
            .collect();
        CatalogueSnapshot::new(snapshot.generation, models)
    }
}

pub struct ResolveModelSelectionUseCase {
    store: CatalogueSnapshotStore,
}

impl ResolveModelSelectionUseCase {
    pub fn new(store: CatalogueSnapshotStore) -> Self {
        Self { store }
    }

    pub fn resolve(&self, reference: &ModelRef) -> Result<ModelDescriptor, SelectionFailure> {
        let snapshot = self.store.current();
        let Some(model) = snapshot.find(reference) else {
            return Err(SelectionFailure::UnknownModel);
        };
        if !model.availability.runnable() {
            return Err(SelectionFailure::Unavailable {
                reasons: model.availability.reasons().to_vec(),
            });
        }
        Ok(model.clone())
    }
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
