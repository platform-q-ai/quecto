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
    let builtin = BuiltinCatalogueSource;
    let user_file = ModelsFileCatalogueSource::new(base_dir);
    // Credential status comes from the same parsed configuration the sources
    // feed from; a broken file simply configures nothing extra.
    let file_records =
        ModelRegistry::load_file_records(&base_dir.join("models.json")).unwrap_or_default();
    let builtin_registry = ModelRegistry::builtin();
    let credentials = RegistryCredentialStatus::from_records(
        builtin_registry.models().iter().chain(&file_records),
    );
    let resolved =
        ResolveCatalogueUseCase.resolve_and_publish(&[&builtin, &user_file], &credentials, &store);
    (store, resolved)
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
