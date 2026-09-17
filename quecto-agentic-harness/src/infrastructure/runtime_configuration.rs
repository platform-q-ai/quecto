//! The file-backed runtime configuration of one run (#1849), as the reload
//! use case's source: the config file the run selected and the base
//! directory's `models.json` behind the ADR-0002 change gate, rebuilt into
//! a provider runtime plus the persisted tool-policy baseline by one
//! `Config` read and the composition-supplied provider builder.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::application::catalogue::ports::{ReloadedConfiguration, RuntimeConfigurationSource};
use crate::application::providers::ports::LlmProvider;
use crate::infrastructure::config::Config;
use crate::infrastructure::reload::{ReloadSource, RuntimeReload};

/// Composes and publishes the provider runtime for a config and base
/// directory (composition's `build_agent_provider`); every rebuild goes
/// through the same function startup did.
pub type ProviderBuilder =
    fn(&Config, &Path, &reqwest::Client) -> Result<Arc<dyn LlmProvider>, String>;

/// The inputs a rebuild composes from, owned so the rebuild can run on its
/// own OS thread.
#[derive(Clone)]
struct RebuildInputs {
    config_path: PathBuf,
    base_dir: PathBuf,
    env_overrides: HashMap<String, String>,
    http_client: reqwest::Client,
    build_provider: ProviderBuilder,
}

impl RebuildInputs {
    /// One `Config` read feeds both the provider composition and the
    /// tool-policy baseline (fix (a), #1849: formerly parsed twice).
    fn rebuild(&self) -> Result<ReloadedConfiguration, String> {
        let config =
            Config::load_with_env(self.config_path.to_str().unwrap_or(""), &self.env_overrides)
                .map_err(|error| error.to_string())?;
        let provider = (self.build_provider)(&config, &self.base_dir, &self.http_client)?;
        let tool_policy = config
            .tools
            .policy
            .entries
            .into_iter()
            .map(|(stable_id, entry)| (stable_id, entry.scope))
            .collect();
        Ok(ReloadedConfiguration {
            provider,
            tool_policy,
        })
    }
}

pub struct FileRuntimeConfiguration {
    inputs: RebuildInputs,
    gate: RuntimeReload,
}

impl FileRuntimeConfiguration {
    /// Watch `config_path` and `<base_dir>/models.json`, seeded from their
    /// current content so only later edits count as changes.
    pub fn seeded(
        config_path: PathBuf,
        base_dir: PathBuf,
        env_overrides: HashMap<String, String>,
        http_client: reqwest::Client,
        build_provider: ProviderBuilder,
    ) -> Self {
        let mut gate = RuntimeReload::new(vec![
            ReloadSource::new(config_path.clone()),
            ReloadSource::new(base_dir.join("models.json")),
        ]);
        gate.seed();
        Self {
            inputs: RebuildInputs {
                config_path,
                base_dir,
                env_overrides,
                http_client,
                build_provider,
            },
            gate,
        }
    }
}

impl RuntimeConfigurationSource for FileRuntimeConfiguration {
    fn changed(&mut self) -> bool {
        self.gate.sources_changed()
    }

    /// Fingerprints are observed before the read, so a change made after
    /// this rebuild is still detected and one made before it is consumed —
    /// a forced reload never prompts a second rebuild at the next poll
    /// (fix (b), #1849). The composition runs on its own OS thread, off the
    /// caller's async runtime, as startup composition does.
    fn rebuild(&mut self) -> Result<ReloadedConfiguration, String> {
        self.gate.seed();
        let inputs = self.inputs.clone();
        std::thread::spawn(move || inputs.rebuild())
            .join()
            .map_err(|_| "provider reload worker panicked".to_string())?
    }
}

impl std::fmt::Debug for FileRuntimeConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileRuntimeConfiguration")
            .field("config_path", &self.inputs.config_path)
            .field("base_dir", &self.inputs.base_dir)
            .finish_non_exhaustive()
    }
}
