//! Resolve the effective catalogue and publish one immutable generation.

use crate::application::catalogue::dto::resolved_catalogue::ResolvedCatalogue;
use crate::application::catalogue::ports::catalogue_source::{
    CatalogueSource, CatalogueSourceError,
};
use crate::application::catalogue::ports::credential_status::CredentialStatusPort;
use crate::application::catalogue::services::snapshot_store::CatalogueSnapshotStore;
use crate::domain::catalogue::services::availability::{
    derive_availability, refused_availability, transport_has_adapter,
};
use crate::domain::catalogue::services::resolution::resolve_catalogue;

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
        let mut skipped = Vec::new();
        let mut fresh_loads = 0usize;
        for source in sources {
            match source.load() {
                Ok(loaded) => {
                    skipped.extend(
                        loaded
                            .skipped
                            .into_iter()
                            .map(|record| (source.id().to_string(), record)),
                    );
                    store.remember_layer(source.id(), &loaded.entries);
                    fresh_loads += 1;
                    layers.push((source.layer(), loaded.entries));
                }
                Err(error) => {
                    source_errors.push(CatalogueSourceError {
                        source: source.id().to_string(),
                        error,
                    });
                    // Per-source retention (#1581 review): a source that
                    // fails to load degrades to its own last-good entries on
                    // this store, so the other layers' valid updates still
                    // publish (#1575, AC4b — a malformed user file keeps its
                    // last valid contribution, while an unrelated edit or a
                    // broken discovery cache never freezes the catalogue).
                    // A source that never loaded on this store contributes
                    // nothing (malformed-source isolation keeps built-ins
                    // resolving on a broken first read).
                    if let Some(entries) = store.last_good_layer(source.id()) {
                        layers.push((source.layer(), entries));
                    }
                }
            }
        }
        // When every source failed there is nothing fresh to describe: the
        // last valid generation is retained wholesale (never republished as
        // a spurious new generation) and the errors are surfaced.
        if fresh_loads == 0 && !source_errors.is_empty() {
            return ResolvedCatalogue {
                snapshot: store.current(),
                rejected: Vec::new(),
                source_errors,
                skipped,
            };
        }
        for (_, entries) in &mut layers {
            for entry in entries {
                let credential_available = credentials.credential_available(entry);
                entry.model.availability = derive_availability(
                    entry.provider.transport.clone(),
                    transport_has_adapter(&entry.provider.transport),
                    credential_available,
                );
                // #2435: a model the provider refused for the account in
                // use stays unavailable while the refusal is held.
                if let Some(reason) = store.refusal(entry.reference()) {
                    entry.model.availability =
                        refused_availability(&entry.model.availability, reason);
                }
            }
        }
        // Read-increment-resolve-publish happens under one write lock so a
        // concurrent resolve on the same store can never tag a different
        // snapshot with the same generation.
        let (snapshot, rejected) = store.publish_next(|generation| {
            let resolution = resolve_catalogue(generation, layers);
            (resolution.snapshot, resolution.rejected)
        });
        ResolvedCatalogue {
            snapshot,
            rejected,
            source_errors,
            skipped,
        }
    }
}

#[cfg(test)]
#[path = "resolve_catalogue_tests.rs"]
mod tests;
