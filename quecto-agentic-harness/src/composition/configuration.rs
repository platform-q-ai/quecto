//! Configuration composition (#1966, #2024): the configuration use cases
//! over the filesystem document store, the `Config` schema and the
//! persistent overlay trust store. `main` hands
//! [`build_configuration_handles`] to the CLI entry point; the interface
//! only invokes the handles it receives. [`build_config_loader`] is the
//! same graph as one reloadable function for the runtime-configuration
//! source.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use crate::application::configuration::dto::ConfigSelection;
use crate::application::configuration::use_cases::{
    PatchConfiguration, ReadConfiguration, ResolveEffectiveConfig, SelectConfig, TrustConfigOverlay,
};
use crate::infrastructure::config::loaders::FilesystemConfigDocumentStore;
use crate::infrastructure::config::mapping::{ConfigValidatorAdapter, realize_config};
use crate::infrastructure::config::persistence::{
    PersistentOverlayTrustStore, TRUST_RECORD_FILE_NAME,
};
use crate::infrastructure::config::writer::JsonDocumentWriter;
use crate::infrastructure::reload::ReloadSource;
use crate::infrastructure::runtime_configuration::ConfigLoader;
use crate::interface::cli::configuration_handles::{
    ConfigRealizer, ConfigurationEnvironment, ConfigurationHandles,
};

pub fn build_configuration_handles(env: &ConfigurationEnvironment) -> ConfigurationHandles {
    let store = Arc::new(FilesystemConfigDocumentStore);
    let writer = Arc::new(JsonDocumentWriter);
    let validator = Arc::new(ConfigValidatorAdapter);
    let trust = Arc::new(PersistentOverlayTrustStore::for_base_dir(
        &env.base_dir,
        env.prompt_for_trust,
    ));
    let resolve = Arc::new(ResolveEffectiveConfig::new(
        store.clone(),
        validator.clone(),
        trust.clone(),
    ));
    ConfigurationHandles {
        select: Arc::new(SelectConfig::new()),
        read: Arc::new(ReadConfiguration::new(store.clone(), resolve.clone())),
        patch: Arc::new(PatchConfiguration::new(
            store.clone(),
            writer,
            validator.clone(),
            trust.clone(),
            resolve.clone(),
        )),
        trust: Arc::new(TrustConfigOverlay::new(store, validator, trust)),
        resolve,
        realize: realizer(&env.base_dir),
    }
}

/// The document → `Config` step as a handle bound to `base_dir`, so the
/// interface never reaches into `infrastructure::config::mapping` itself.
fn realizer(base_dir: &Path) -> ConfigRealizer {
    let base_dir = base_dir.to_path_buf();
    Arc::new(move |document, env_overrides| realize_config(document, env_overrides, &base_dir))
}

/// The run's configuration as one reloadable read (#2024): the selected
/// layers resolved (a never-prompting trust decision — a reload runs on the
/// dispatch loop, not a terminal), realized into a `Config` with
/// `env_overrides` applied and bound to `base_dir`.
pub fn build_config_loader(
    base_dir: &Path,
    selection: ConfigSelection,
    env_overrides: HashMap<String, String>,
) -> ConfigLoader {
    let base_dir = base_dir.to_path_buf();
    let handles = build_configuration_handles(&ConfigurationEnvironment {
        base_dir: base_dir.clone(),
        prompt_for_trust: false,
    });
    Arc::new(move || {
        let effective = handles
            .resolve
            .execute(&selection)
            .map_err(|error| error.to_string())?;
        // A reload has no terminal: an overlay that became untrusted (a
        // hand edit mid-run) drops out of the effective configuration, and
        // the operator's log is the only place that says so.
        for line in effective.sources.diagnostics() {
            tracing::warn!(target: "quecto::configuration", "{line}");
        }
        realize_config(effective.document, &env_overrides, &base_dir)
    })
}

/// The sources a reload must watch for `selection` (#2024): the base file
/// (required: its removal keeps the last-good runtime); when the selection
/// has an overlay location, the overlay (optional: its creation or removal
/// is a change); and, only when an overlay file exists as the watch is
/// seeded, the per-user trust record (optional: `quecto config trust`, or a
/// `config set` that re-records trust, takes effect on the next reload).
/// A session with no overlay is not rebuilt every time some other
/// repository's overlay is trusted — the record is one file for every
/// overlay on the host.
pub fn watched_config_sources(base_dir: &Path, selection: &ConfigSelection) -> Vec<ReloadSource> {
    let mut watched = vec![ReloadSource::new(selection.path())];
    if let Some(overlay) = selection.overlay_path() {
        watched.push(ReloadSource::optional(overlay));
        if overlay.exists() {
            watched.push(ReloadSource::optional(
                base_dir.join(TRUST_RECORD_FILE_NAME),
            ));
        }
    }
    watched
}

#[cfg(test)]
#[path = "configuration_tests.rs"]
mod tests;
