//! Catalogue resolution: entry validation and the deterministic merge of
//! source layers into a snapshot.

use std::collections::HashMap;

use crate::domain::catalogue::entities::entry::CatalogueEntry;
use crate::domain::catalogue::entities::snapshot::{
    CatalogueResolution, CatalogueSnapshot, RejectedEntry,
};
use crate::domain::catalogue::value_objects::error::CatalogueDomainError;
use crate::domain::catalogue::value_objects::identity::ModelRef;
use crate::domain::catalogue::value_objects::precedence::SourceLayer;

/// Validate a single entry: the model reference must name the entry's own
/// provider, and declared limits must be non-zero.
pub fn validate_entry(entry: &CatalogueEntry) -> Result<(), CatalogueDomainError> {
    if entry.model.reference.provider() != &entry.provider.id {
        return Err(CatalogueDomainError::ProviderMismatch {
            entry_provider: entry.provider.id.as_str().to_string(),
            model_provider: entry.model.reference.provider().as_str().to_string(),
        });
    }
    if entry.model.capabilities.context_window == 0 {
        return Err(CatalogueDomainError::ZeroLimit(
            "context_window".to_string(),
        ));
    }
    if entry.model.capabilities.max_output_tokens == 0 {
        return Err(CatalogueDomainError::ZeroLimit(
            "max_output_tokens".to_string(),
        ));
    }
    Ok(())
}

/// Deterministically resolve source layers into a snapshot.
///
/// Rules (see module docs): layers are ordered by [`SourceLayer`] precedence
/// regardless of the order supplied; entries upsert by stable [`ModelRef`]
/// identity, an override keeping the overridden entry's ordering position;
/// invalid entries are recorded in `rejected` and skipped without affecting
/// any other entry. Within a single layer, a later duplicate of the same
/// reference wins (last-writer within the layer).
pub fn resolve_catalogue(
    generation: u64,
    mut layers: Vec<(SourceLayer, Vec<CatalogueEntry>)>,
) -> CatalogueResolution {
    // Precedence is a property of the layer, not the input order. The sort is
    // stable, so multiple inputs for the same layer keep their relative order
    // and last-writer-wins applies within a layer.
    layers.sort_by_key(|(layer, _)| *layer);

    let mut entries: Vec<CatalogueEntry> = Vec::new();
    let mut positions: HashMap<ModelRef, usize> = HashMap::new();
    let mut rejected: Vec<RejectedEntry> = Vec::new();

    for (layer, layer_entries) in layers {
        for entry in layer_entries {
            if let Err(error) = validate_entry(&entry) {
                rejected.push(RejectedEntry {
                    entry,
                    layer,
                    error,
                });
                continue;
            }
            match positions.get(entry.reference()) {
                Some(&position) => entries[position] = entry,
                None => {
                    positions.insert(entry.reference().clone(), entries.len());
                    entries.push(entry);
                }
            }
        }
    }

    CatalogueResolution {
        snapshot: CatalogueSnapshot {
            generation,
            entries,
        },
        rejected,
    }
}

#[cfg(test)]
#[path = "resolution_tests.rs"]
mod tests;
