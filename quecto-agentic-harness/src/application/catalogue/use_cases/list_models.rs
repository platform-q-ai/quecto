//! List available models (#1845): inspect the configured/built-in catalogue
//! without discovery network traffic — the query behind UDS `list_models`
//! and, through it, the TUI model list.
//!
//! Every call loads the inputs and republishes the effective snapshot into
//! the store, so the listing is level with on-disk edits; the network-only
//! refresh is a different use case. Sources that fail to load are reported
//! rather than listed over; the generation the healthy layers produced is
//! still published.

use std::sync::Arc;

use crate::application::catalogue::dto::{
    ListedModel, ListingDiagnostic, ModelCatalogueListing, ModelListingOutcome,
};
use crate::application::catalogue::ports::CatalogueInputsLoader;
use crate::application::catalogue::{CatalogueSnapshotStore, ResolveCatalogueUseCase};

pub struct ListModels {
    inputs: Arc<dyn CatalogueInputsLoader>,
    store: CatalogueSnapshotStore,
}

impl ListModels {
    pub fn new(inputs: Arc<dyn CatalogueInputsLoader>, store: CatalogueSnapshotStore) -> Self {
        Self { inputs, store }
    }

    pub fn execute(&self) -> ModelListingOutcome {
        let loaded = self.inputs.load();
        let resolved = ResolveCatalogueUseCase.resolve_and_publish(
            &loaded.sources(),
            loaded.credentials(),
            &self.store,
        );
        if !resolved.source_errors.is_empty() {
            return ModelListingOutcome::SourcesUnavailable(resolved.source_errors);
        }
        // The generation this call just published — never a store read that
        // a concurrent publish could have moved past.
        let snapshot = resolved.snapshot;
        let rejected = resolved
            .rejected
            .iter()
            .map(|rejection| ListingDiagnostic {
                model: rejection.entry.reference().qualified_id(),
                reason: rejection.error.to_string(),
            })
            .chain(
                resolved
                    .skipped
                    .iter()
                    .map(|(source, record)| ListingDiagnostic {
                        model: record.record.clone(),
                        reason: format!("{source}: {}", record.error),
                    }),
            )
            .collect();
        ModelListingOutcome::Listed(ModelCatalogueListing {
            generation: snapshot.generation(),
            models: snapshot
                .entries()
                .iter()
                .map(|entry| ListedModel {
                    runnable: entry.model.availability.is_runnable(),
                    entry: entry.clone(),
                })
                .collect(),
            rejected,
        })
    }
}

impl std::fmt::Debug for ListModels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListModels").finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "list_models_tests.rs"]
mod tests;
