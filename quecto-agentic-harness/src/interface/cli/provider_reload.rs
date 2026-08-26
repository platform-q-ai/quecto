//! Provider reload wiring for ADR-0002 Phase 2b.

use std::collections::HashMap;
use std::path::PathBuf;
#[cfg(any(test, feature = "test-support"))]
use std::sync::Arc;

use crate::application::provider_runtime::CatalogueRuntimeSnapshot;
#[cfg(any(test, feature = "test-support"))]
use crate::domain::provider::LlmProvider;
use crate::infrastructure::config::Config;
use crate::infrastructure::reload::{ReloadResult, ReloadSource, RuntimeReload};

use crate::interface::catalogue_runtime::build_runtime_snapshot;

pub type ProviderReload = RuntimeReload<CatalogueRuntimeSnapshot>;

/// Owned inputs used to rebuild the provider set after a config reload.
///
/// This deliberately owns its fields so UDS dispatch can borrow the reload gate,
/// run the rebuild closure, then borrow the agent for the swap without capturing
/// the whole dispatch context.
#[derive(Clone)]
pub struct ProviderReloadInputs {
    pub config_path: PathBuf,
    pub base_dir: PathBuf,
    pub env_overrides: HashMap<String, String>,
    pub http_client: reqwest::Client,
}

impl ProviderReloadInputs {
    pub fn new(
        config_path: PathBuf,
        base_dir: PathBuf,
        env_overrides: HashMap<String, String>,
        http_client: reqwest::Client,
    ) -> Self {
        Self {
            config_path,
            base_dir,
            env_overrides,
            http_client,
        }
    }

    pub async fn rebuild_blocking(&self) -> Result<CatalogueRuntimeSnapshot, String> {
        let inputs = self.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        std::thread::spawn(move || {
            let _ = tx.send(inputs.rebuild_on_current_thread());
        });
        rx.await
            .map_err(|_| "provider reload worker panicked".to_string())?
    }

    fn rebuild_on_current_thread(&self) -> Result<CatalogueRuntimeSnapshot, String> {
        let config =
            Config::load_with_env(self.config_path.to_str().unwrap_or(""), &self.env_overrides)
                .map_err(|e| e.to_string())?;
        build_runtime_snapshot(&config, &self.base_dir, &self.http_client, 0)
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn seeded_provider_reload(
    config_path: impl Into<PathBuf>,
    initial_provider: Arc<dyn LlmProvider>,
) -> ProviderReload {
    let catalogue = crate::domain::catalogue::CatalogueSnapshot::new(
        0,
        initial_provider.model_descriptors().unwrap_or(&[]).to_vec(),
    )
    .with_open_providers(
        initial_provider
            .open_provider_names()
            .into_iter()
            .filter_map(|name| crate::domain::catalogue::ProviderId::new(name).ok())
            .collect(),
    );
    seeded_provider_reload_with_base(
        config_path,
        None,
        CatalogueRuntimeSnapshot {
            provider: initial_provider,
            catalogue,
        },
    )
}

/// Seed the reload gate with the runtime the session is actually running, so
/// the retained last-good generation is the published one — not a reconstruction
/// that would answer differently about what the runtime can route.
pub fn seeded_provider_reload_with_base(
    config_path: impl Into<PathBuf>,
    base_dir: Option<PathBuf>,
    initial_runtime: CatalogueRuntimeSnapshot,
) -> ProviderReload {
    let mut sources = vec![ReloadSource::new(config_path.into())];
    if let Some(base_dir) = base_dir {
        sources.push(ReloadSource::new(base_dir.join("models.json")));
    }
    let mut reload = RuntimeReload::new(sources);
    reload.seed(initial_runtime);
    reload
}

/// The last-good runtime is retained on failure; the error is returned so
/// consumers can report that the catalogue on disk is currently unusable.
pub async fn poll_provider_reload(
    reload: Option<&mut ProviderReload>,
    inputs: Option<&ProviderReloadInputs>,
) -> Option<Result<ReloadResult<CatalogueRuntimeSnapshot>, String>> {
    let (Some(reload), Some(inputs)) = (reload, inputs) else {
        return None;
    };
    if !reload.sources_changed() {
        return Some(Ok(ReloadResult::Unchanged));
    }
    match inputs.rebuild_blocking().await {
        Ok(mut runtime) => {
            runtime.catalogue.generation = next_generation(reload);
            Some(Ok(reload.record_reloaded(runtime)))
        }
        Err(err) => {
            tracing::warn!(target: "reload", error = %err, "reload rebuild failed; keeping last-good");
            Some(Err(err))
        }
    }
}

fn next_generation(reload: &ProviderReload) -> u64 {
    reload
        .last_good()
        .map(CatalogueRuntimeSnapshot::generation)
        .unwrap_or(0)
        .saturating_add(1)
}

pub async fn force_provider_reload(
    reload: Option<&mut ProviderReload>,
    inputs: Option<&ProviderReloadInputs>,
) -> Option<Result<ReloadResult<CatalogueRuntimeSnapshot>, String>> {
    let (Some(reload), Some(inputs)) = (reload, inputs) else {
        return None;
    };
    // Re-seed the change gate before reading the sources, so a forced rebuild
    // does not make the next poll repeat it for content it already observed —
    // while a write that lands during the rebuild is still seen by that poll.
    reload.sources_changed();
    match inputs.rebuild_blocking().await {
        Ok(mut runtime) => {
            runtime.catalogue.generation = next_generation(reload);
            Some(Ok(reload.record_reloaded(runtime)))
        }
        Err(err) => {
            tracing::warn!(target: "reload", error = %err, "forced reload failed; keeping last-good");
            Some(Err(err))
        }
    }
}
