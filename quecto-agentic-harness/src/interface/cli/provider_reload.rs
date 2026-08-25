//! Provider reload wiring for ADR-0002 Phase 2b.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::application::provider_runtime::CatalogueRuntimeSnapshot;
use crate::domain::catalogue::{CatalogueSnapshot, ModelDescriptor};
use crate::domain::provider::LlmProvider;
use crate::infrastructure::config::Config;
use crate::infrastructure::reload::{ReloadResult, ReloadSource, RuntimeReload};

use crate::infrastructure::provider_runtime_catalogue::build_agent_provider_with_descriptors;

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
        let (provider, descriptors) =
            build_agent_provider_with_descriptors(&config, &self.base_dir, &self.http_client)?;
        Ok(CatalogueRuntimeSnapshot {
            provider,
            catalogue: CatalogueSnapshot::new(0, descriptors),
        })
    }
}

#[cfg(any(test, feature = "test-support"))]
pub fn seeded_provider_reload(
    config_path: impl Into<PathBuf>,
    initial_provider: Arc<dyn LlmProvider>,
) -> ProviderReload {
    seeded_provider_reload_with_base(config_path, None, initial_provider)
}

pub fn seeded_provider_reload_with_base(
    config_path: impl Into<PathBuf>,
    base_dir: Option<PathBuf>,
    initial_provider: Arc<dyn LlmProvider>,
) -> ProviderReload {
    let mut sources = vec![ReloadSource::new(config_path.into())];
    if let Some(base_dir) = base_dir {
        sources.push(ReloadSource::new(base_dir.join("models.json")));
    }
    let mut reload = RuntimeReload::new(sources);
    reload.seed(CatalogueRuntimeSnapshot {
        provider: initial_provider,
        catalogue: CatalogueSnapshot::new(0, Vec::<ModelDescriptor>::new()),
    });
    reload
}

pub async fn poll_provider_reload(
    reload: Option<&mut ProviderReload>,
    inputs: Option<&ProviderReloadInputs>,
) -> Option<ReloadResult<CatalogueRuntimeSnapshot>> {
    let (Some(reload), Some(inputs)) = (reload, inputs) else {
        return None;
    };
    if !reload.sources_changed() {
        return Some(ReloadResult::Unchanged);
    }
    match inputs.rebuild_blocking().await {
        Ok(mut runtime) => {
            runtime.catalogue.generation = next_generation(reload);
            Some(reload.record_reloaded(runtime))
        }
        Err(err) => {
            tracing::warn!(target: "reload", error = %err, "reload rebuild failed; keeping last-good");
            Some(ReloadResult::Unchanged)
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
