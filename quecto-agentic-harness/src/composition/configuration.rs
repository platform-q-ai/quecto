//! Configuration composition (#1966, #2024): the configuration use cases
//! over the filesystem document store, the `Config` schema and the
//! persistent overlay trust store. `main` hands
//! [`build_configuration_handles`] to the CLI entry point; the interface
//! only invokes the handles it receives. [`build_config_loader`] is the
//! same graph as one reloadable function for the runtime-configuration
//! source.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
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
use crate::infrastructure::runtime_configuration::ConfigLoader;
use crate::interface::cli::configuration_handles::{
    ConfigurationEnvironment, ConfigurationHandles,
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
        )),
        trust: Arc::new(TrustConfigOverlay::new(store, validator, trust)),
        resolve,
    }
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
        for line in crate::interface::cli::config_loading::layer_diagnostics(&effective.sources) {
            tracing::warn!(target: "quecto::configuration", "{line}");
        }
        realize_config(effective.document, &env_overrides, &base_dir)
    })
}

/// The files a reload must watch for `selection` (#2024): the base file,
/// the overlay when one applies, and the overlay trust record (so
/// `quecto config trust` takes effect on the next reload).
pub fn watched_config_files(base_dir: &Path, selection: &ConfigSelection) -> Vec<PathBuf> {
    let mut watched = vec![selection.path().to_path_buf()];
    if let Some(overlay) = selection.overlay_path() {
        watched.push(overlay.to_path_buf());
        watched.push(base_dir.join(TRUST_RECORD_FILE_NAME));
    }
    watched
}

#[cfg(test)]
#[path = "configuration_tests.rs"]
mod tests;
