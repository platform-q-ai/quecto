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
    BuiltinCatalogueSource, ModelsFileCatalogueSource, RegistryCredentialStatus,
    UserOverrideCatalogueSource, apply_overrides, entries_from_records, snapshot_store_for,
    user_file_entries,
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
    /// The user's stable-ID `overrides` section as the `UserOverride` layer
    /// (#1575): patched full entries plus per-override diagnostics.
    user_overrides: UserOverrideCatalogueSource,
    /// Persisted discovery caches (generated data), fed in as the discovered
    /// layer so explicit refreshes participate in normal precedence. Loading
    /// them touches no network.
    discovered: Vec<crate::infrastructure::catalogue_discovery::DiscoverySourceCache>,
    pub(crate) credentials: RegistryCredentialStatus,
    /// The parsed user-file records (or the parse error), kept so runtime
    /// composition can build its effective registry from the same read.
    file_records: Result<Vec<crate::infrastructure::model_registry::ModelRecord>, String>,
    /// Per-provider connection defaults from the same parse, so the refresh
    /// path can build discovery sources without re-reading `models.json`.
    provider_defaults: Result<
        Vec<(
            String,
            crate::infrastructure::model_registry::ProviderDefaults,
        )>,
        String,
    >,
    /// Records synthesized for discovered-cache models under configured
    /// providers (connection/auth from the provider's defaults), so a
    /// discovered model is credentialed and routable like a listed one.
    discovered_records: Vec<crate::infrastructure::model_registry::ModelRecord>,
    /// Full records produced by applying the user's stable-ID overrides to
    /// their base records, so overridden connection/credential references are
    /// also routable and credential-checked.
    override_records: Vec<crate::infrastructure::model_registry::ModelRecord>,
}

impl CatalogueInputs {
    /// models.json is read and parsed exactly once per load: the same parse
    /// feeds the user-defined source layer, credential status, discovery
    /// provider defaults, and the effective registry, so every consumer
    /// describes one on-disk state (and a resolve costs one file read).
    pub(crate) fn load(base_dir: &Path) -> Self {
        let config = ModelRegistry::load_registry_config(&base_dir.join("models.json"))
            .map_err(|error| error.to_string());
        let file_load = config
            .as_ref()
            .map(|c| c.records.clone())
            .map_err(Clone::clone);
        let user_entries = config
            .as_ref()
            .map(|c| user_file_entries(&c.records, &c.unsupported, &c.skipped))
            .map_err(Clone::clone);
        let overrides = config
            .as_ref()
            .map(|c| (c.overrides.clone(), c.unsupported.clone()))
            .map_err(Clone::clone);
        let provider_defaults = config.map(|c| c.providers);
        let discovered =
            crate::infrastructure::catalogue_discovery::discovery_cache_sources(base_dir);
        let discovered_records = synthesize_discovered_records(
            &discovered,
            provider_defaults.as_deref().unwrap_or_default(),
            file_load.as_deref().unwrap_or_default(),
        );
        let builtin_registry = ModelRegistry::builtin();
        // A malformed parse propagates as the override layer's error (so its
        // last-good entries are retained by the resolve) instead of
        // publishing an empty override layer.
        let (override_records, override_entries) = match &overrides {
            Ok((overrides, unsupported)) => {
                let applied = apply_overrides(
                    overrides,
                    file_load.as_deref().unwrap_or_default(),
                    &discovered_records,
                    &builtin_registry,
                    unsupported,
                );
                let mut entries = entries_from_records(&applied.records);
                entries.entries.extend(applied.unsupported_entries);
                entries.skipped.extend(applied.skipped);
                (applied.records, Ok(entries))
            }
            Err(error) => (Vec::new(), Err(error.clone())),
        };
        let credentials = RegistryCredentialStatus::from_records(
            builtin_registry
                .models()
                .iter()
                .chain(file_load.as_deref().unwrap_or_default())
                .chain(discovered_records.iter())
                .chain(override_records.iter()),
        );
        Self {
            builtin: BuiltinCatalogueSource,
            user_file: ModelsFileCatalogueSource::preloaded(user_entries),
            user_overrides: UserOverrideCatalogueSource::preloaded(override_entries),
            discovered,
            credentials,
            file_records: file_load,
            provider_defaults,
            discovered_records,
            override_records,
        }
    }

    /// The effective model registry (built-in + user file + discovered-cache
    /// records under configured providers) from this load's records, so
    /// catalogue and router always describe one on-disk state and a
    /// discovered model the catalogue publishes as runnable also has a route.
    /// User-file records win over synthesized discovered ones (upsert order).
    pub(crate) fn effective_registry(
        &self,
    ) -> Result<crate::infrastructure::model_registry::ModelRegistry, String> {
        self.file_records.clone().map(|mut records| {
            let mut merged = self.discovered_records.clone();
            merged.append(&mut records);
            // Overrides win over both (upsert order): an overridden
            // credential reference or limit is what the runtime routes with.
            merged.extend(self.override_records.iter().cloned());
            crate::infrastructure::model_registry::ModelRegistry::from_file_records(merged)
        })
    }

    /// The per-provider connection defaults from this load's parse (or the
    /// parse error), for composing the refresh path from the same read.
    pub(crate) fn provider_defaults(
        &self,
    ) -> &Result<
        Vec<(
            String,
            crate::infrastructure::model_registry::ProviderDefaults,
        )>,
        String,
    > {
        &self.provider_defaults
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
        sources.push(&self.user_overrides);
        sources
    }

    /// Providers that already have a persisted discovery cache feeding the
    /// discovered layer via [`CatalogueInputs::sources`].
    pub(crate) fn discovered_providers(&self) -> Vec<&str> {
        self.discovered.iter().map(|c| c.provider()).collect()
    }
}

/// Synthesize model records for discovered-cache models whose provider is
/// configured in `models.json`: connection and auth come from the provider's
/// defaults, so a discovered model is credentialed and routable exactly like
/// a listed one (slice-4 review — the legacy discover flow guaranteed this by
/// rewriting `models.json`; the cache-only flow must not lose it). Models the
/// user already lists are skipped (the file's record wins), as are caches for
/// providers no longer configured.
fn synthesize_discovered_records(
    caches: &[crate::infrastructure::catalogue_discovery::DiscoverySourceCache],
    provider_defaults: &[(
        String,
        crate::infrastructure::model_registry::ProviderDefaults,
    )],
    file_records: &[crate::infrastructure::model_registry::ModelRecord],
) -> Vec<crate::infrastructure::model_registry::ModelRecord> {
    use crate::application::catalogue::CatalogueSource as _;
    let mut records = Vec::new();
    for cache in caches {
        let Some((key, defaults)) = provider_defaults
            .iter()
            .find(|(key, _)| key == cache.provider())
        else {
            continue;
        };
        let Ok(entries) = cache.load() else { continue };
        for entry in entries.entries {
            let id = entry.model.reference.model().as_str();
            if file_records
                .iter()
                .any(|r| r.provider == *key && r.id == id)
            {
                continue;
            }
            records.push(defaults.record_for(key, id, entry.model.display_name.as_deref()));
        }
    }
    records
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
