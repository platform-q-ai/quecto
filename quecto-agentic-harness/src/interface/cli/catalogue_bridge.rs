//! Interface-level composition of the effective-catalogue read path (epic
//! #1193, slice 2): wires the infrastructure source/credential adapters into
//! the application resolve use case and exposes snapshot-backed reads to the
//! CLI/UDS/REPL surfaces. Lives in the interface layer because it is the only
//! layer allowed to see both application use cases and infrastructure
//! adapters.

use std::path::Path;

use crate::application::catalogue::{ResolveCatalogueUseCase, ResolvedCatalogue, model_limits_in};
use crate::application::ports::CatalogueSnapshotStore;
use crate::infrastructure::catalogue_registry::{
    BuiltinCatalogueSource, ModelsFileCatalogueSource, RegistryCredentialStatus, snapshot_store_for,
};
use crate::infrastructure::model_registry::ModelRegistry;

/// Run the resolve-effective-catalogue use case over the real sources for
/// `base_dir` and publish into its shared store. Startup calls this once to
/// publish the initial generation; the read surfaces call it again to stay
/// level with on-disk edits until explicit refresh arrives (epic #1193 slices
/// 4-5). No network is touched.
pub fn resolve_and_publish_for(base_dir: &Path) -> (CatalogueSnapshotStore, ResolvedCatalogue) {
    let store = snapshot_store_for(base_dir);
    let inputs = CatalogueInputs::load(base_dir);
    let resolved =
        ResolveCatalogueUseCase.resolve_and_publish(&inputs.sources(), &inputs.credentials, &store);
    (store, resolved)
}

/// The real catalogue sources and credential status for one base directory,
/// loaded once so a resolve (or runtime composition) reads one on-disk state.
pub(crate) struct CatalogueInputs {
    builtin: BuiltinCatalogueSource,
    user_file: ModelsFileCatalogueSource,
    /// Persisted discovery caches (generated data), fed in as the discovered
    /// layer so explicit refreshes participate in normal precedence. Loading
    /// them touches no network.
    discovered: Vec<crate::infrastructure::catalogue_discovery::DiscoverySourceCache>,
    pub(crate) credentials: RegistryCredentialStatus,
    /// The parsed user-file records (or the parse error), kept so runtime
    /// composition can build its effective registry from the same read.
    file_records: Result<Vec<crate::infrastructure::model_registry::ModelRecord>, String>,
}

impl CatalogueInputs {
    /// models.json is read and parsed exactly once per load: the same parse
    /// feeds both the user-defined source layer and credential status, so the
    /// published entries and their availability always describe one on-disk
    /// state (and a resolve costs one file read, not two).
    pub(crate) fn load(base_dir: &Path) -> Self {
        let file_load = ModelRegistry::load_file_records(&base_dir.join("models.json"))
            .map_err(|error| error.to_string());
        let builtin_registry = ModelRegistry::builtin();
        let credentials = RegistryCredentialStatus::from_records(
            builtin_registry
                .models()
                .iter()
                .chain(file_load.as_deref().unwrap_or_default()),
        );
        Self {
            builtin: BuiltinCatalogueSource,
            user_file: ModelsFileCatalogueSource::preloaded(file_load.clone()),
            discovered: crate::infrastructure::catalogue_discovery::discovery_cache_sources(
                base_dir,
            ),
            credentials,
            file_records: file_load,
        }
    }

    /// The effective model registry (built-in + user file) from this load's
    /// records, so catalogue and router always describe one on-disk state.
    pub(crate) fn effective_registry(
        &self,
    ) -> Result<crate::infrastructure::model_registry::ModelRegistry, String> {
        self.file_records
            .clone()
            .map(crate::infrastructure::model_registry::ModelRegistry::from_file_records)
    }

    pub(crate) fn sources(&self) -> Vec<&dyn crate::application::catalogue::CatalogueSource> {
        let mut sources: Vec<&dyn crate::application::catalogue::CatalogueSource> =
            vec![&self.builtin];
        sources.extend(
            self.discovered
                .iter()
                .map(|c| c as &dyn crate::application::catalogue::CatalogueSource),
        );
        sources.push(&self.user_file);
        sources
    }

    /// Providers that already have a persisted discovery cache feeding the
    /// discovered layer via [`CatalogueInputs::sources`].
    pub(crate) fn discovered_providers(&self) -> Vec<&str> {
        self.discovered.iter().map(|c| c.provider()).collect()
    }
}

/// The per-model limits for a qualified `provider/model` string, read from the
/// published catalogue snapshot: `(output cap, context window)`, each `None`
/// unless explicitly declared. Replaces the legacy private re-parse in
/// `ModelRegistry` so the registry is no longer an independent truth for
/// listing metadata (epic #1193, slice 2). The resolve keeps the legacy
/// freshness (a re-read per call) until explicit refresh lands in slices 4-5;
/// a malformed models.json still falls back to the built-in layer via
/// malformed-source isolation.
pub fn model_limits_from_base_dir(
    base_dir: &Path,
    qualified: &str,
) -> (Option<u32>, Option<usize>) {
    let (store, _) = resolve_and_publish_for(base_dir);
    model_limits_in(&store.current(), qualified)
}

#[cfg(test)]
#[path = "catalogue_bridge_tests.rs"]
mod tests;
