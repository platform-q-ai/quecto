//! Durable tool-policy persistence (#1849): writes the tool-policy
//! mutations the loop applied back into the run's config file as
//! `tools.policy.entries`, the baseline a later reload re-applies.
//! Composition installs it on the loop at build time.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::domain::tool::{ToolPolicyMutationStatus, ToolPolicyReconciliation};
use crate::infrastructure::config::{Config, ToolPolicyEntryConfig};

/// The loop's persistence hook shape (`application::agent_loop::ToolPolicyPersistence`).
pub type ToolPolicyPersistenceFn =
    Arc<dyn Fn(&ToolPolicyReconciliation) -> Result<(), String> + Send + Sync>;

/// A persistence hook that writes every applied mutation into `config_path`.
pub fn tool_policy_persistence_for(config_path: PathBuf) -> ToolPolicyPersistenceFn {
    Arc::new(move |reconciliation| persist_tool_policy_results(&config_path, reconciliation))
}

/// Persist only user preferences already present on disk. `load_with_env`
/// is deliberately not used: environment overrides may contain secrets and
/// must not be serialized back into the durable config file.
pub fn persist_tool_policy_results(
    config_path: &Path,
    reconciliation: &ToolPolicyReconciliation,
) -> Result<(), String> {
    let mut config = Config::load(config_path.to_str().unwrap_or(""))
        .map_err(|e| format!("failed to load config for tool policy persistence: {e}"))?;
    for result in &reconciliation.results {
        if matches!(
            result.status,
            ToolPolicyMutationStatus::Applied | ToolPolicyMutationStatus::AlreadyInState
        ) {
            if let Some(after) = &result.after {
                config.tools.policy.entries.insert(
                    after.stable_id.to_string(),
                    ToolPolicyEntryConfig {
                        scope: result.requested_scope,
                    },
                );
            }
        }
    }
    let parent = config_path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)
        .map_err(|e| format!("failed to create config directory: {e}"))?;
    let json = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("failed to serialize config: {e}"))?;
    std::fs::write(config_path, format!("{json}\n"))
        .map_err(|e| format!("failed to write config: {e}"))
}
