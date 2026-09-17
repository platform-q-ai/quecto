//! The file-backed runtime configuration of one run (#1849), as the reload
//! use case's source: the config file the run selected and the base
//! directory's `models.json` behind the ADR-0002 change gate, rebuilt into
//! a provider runtime plus the persisted tool-policy baseline by one
//! `Config` read and the composition-supplied provider builder. The rebuild
//! runs on the calling thread and blocks it: the caller (the interface's
//! dispatch loop) runs the use case's rebuild phase off its async runtime.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::application::catalogue::ports::{ReloadedConfiguration, RuntimeConfigurationSource};
use crate::application::providers::ports::LlmProvider;
use crate::infrastructure::config::Config;
use crate::infrastructure::reload::{ReloadSource, RuntimeReload};

/// Composition's provider-runtime builder (#1849): composes and publishes
/// the provider runtime for a configuration and base directory and returns
/// its routing provider. Defined once, here, so startup (through the CLI
/// context's alias) and every reload (this source) are handed the same
/// injected function; neither the interface nor this layer composes one.
pub type ProviderRuntimeBuilder =
    fn(&Config, &Path, &reqwest::Client) -> Result<Arc<dyn LlmProvider>, String>;

/// Composition's effective-configuration loader (#2024): the run's
/// selected layers (global file plus trusted overlay, or the explicit
/// file) read, merged, validated and env-overridden into one `Config`.
/// Startup and every reload go through the same injected function.
pub type ConfigLoader = Arc<dyn Fn() -> Result<Config, String> + Send + Sync>;

/// The inputs a rebuild composes from; the builder is the one startup
/// composed through, so every rebuild goes through the same function.
struct RebuildInputs {
    watched: Vec<PathBuf>,
    base_dir: PathBuf,
    load_config: ConfigLoader,
    http_client: reqwest::Client,
    build_provider: ProviderRuntimeBuilder,
}

impl RebuildInputs {
    /// One `Config` read feeds both the provider composition and the
    /// tool-policy baseline (fix (a), #1849: formerly parsed twice).
    fn rebuild(&self) -> Result<ReloadedConfiguration, String> {
        let config = (self.load_config)()?;
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
    /// Watch every configuration file of the run (`config_paths`: the
    /// selected file and, when one applies, the overlay) and
    /// `<base_dir>/models.json`, seeded from their current content so only
    /// later edits count as changes.
    pub fn seeded(
        config_paths: Vec<PathBuf>,
        base_dir: PathBuf,
        load_config: ConfigLoader,
        http_client: reqwest::Client,
        build_provider: ProviderRuntimeBuilder,
    ) -> Self {
        let mut watched = config_paths;
        watched.push(base_dir.join("models.json"));
        let mut gate = RuntimeReload::new(watched.iter().cloned().map(ReloadSource::new).collect());
        gate.seed();
        Self {
            inputs: RebuildInputs {
                watched,
                base_dir,
                load_config,
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
    /// (fix (b), #1849). Blocks the calling thread for the read and the
    /// composition.
    fn rebuild(&mut self) -> Result<ReloadedConfiguration, String> {
        self.gate.seed();
        self.inputs.rebuild()
    }
}

impl std::fmt::Debug for FileRuntimeConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileRuntimeConfiguration")
            .field("watched", &self.inputs.watched)
            .field("base_dir", &self.inputs.base_dir)
            .finish_non_exhaustive()
    }
}
