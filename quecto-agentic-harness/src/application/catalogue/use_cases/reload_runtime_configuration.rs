//! Reload runtime configuration (#1849): apply changed provider/model
//! configuration to a running session without restarting it — the UDS
//! `reload` command (forced) and the pull-based poll before every prompt
//! and `set_model` (ADR-0002: no watcher, no per-turn rebuild).
//!
//! Policy: a poll asks the source whether its files changed and rebuilds
//! only then; a forced reload always rebuilds. A successful rebuild swaps
//! the provider and re-applies the persisted tool policy from the same
//! read, which clears the live-only overlays. A failed rebuild retains the
//! last-good runtime — nothing was published, so nothing is swapped: a poll
//! reports it as unchanged (logged), a forced reload reports the error.

use std::sync::Mutex;

use crate::application::catalogue::dto::ReloadOutcome;
use crate::application::catalogue::ports::{
    ReloadRuntime, ReloadedConfiguration, RuntimeConfigurationSource,
};

pub struct ReloadRuntimeConfiguration {
    /// `None` for a run that has no reloadable configuration (rigs, the
    /// `models` CLI): every reload reports [`ReloadOutcome::NotConfigured`].
    source: Option<Mutex<Box<dyn RuntimeConfigurationSource>>>,
}

impl ReloadRuntimeConfiguration {
    pub fn new(source: Box<dyn RuntimeConfigurationSource>) -> Self {
        Self {
            source: Some(Mutex::new(source)),
        }
    }

    /// A use case with nothing to reload from.
    pub fn unconfigured() -> Self {
        Self { source: None }
    }

    /// Forced reload (UDS `reload`): rebuild regardless of whether anything
    /// changed and report a rebuild failure to the requester.
    pub fn execute(&self, runtime: &mut dyn ReloadRuntime) -> ReloadOutcome {
        let Some(source) = &self.source else {
            return ReloadOutcome::NotConfigured;
        };
        let mut source = source
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match source.rebuild() {
            Ok(configuration) => Self::apply(runtime, configuration),
            Err(error) => {
                tracing::warn!(target: "reload", error = %error, "forced reload failed; keeping last-good");
                ReloadOutcome::Failed(error)
            }
        }
    }

    /// Poll (before a prompt or `set_model`): rebuild only when a watched
    /// file changed; a rebuild failure keeps the last-good runtime silently
    /// for the caller (it is logged) so the turn proceeds.
    pub fn execute_if_changed(&self, runtime: &mut dyn ReloadRuntime) -> ReloadOutcome {
        let Some(source) = &self.source else {
            return ReloadOutcome::NotConfigured;
        };
        let mut source = source
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !source.changed() {
            return ReloadOutcome::Unchanged;
        }
        match source.rebuild() {
            Ok(configuration) => Self::apply(runtime, configuration),
            Err(error) => {
                tracing::warn!(target: "reload", error = %error, "reload rebuild failed; keeping last-good");
                ReloadOutcome::Unchanged
            }
        }
    }

    /// Provider first, then the policy baseline from the same read.
    fn apply(
        runtime: &mut dyn ReloadRuntime,
        configuration: ReloadedConfiguration,
    ) -> ReloadOutcome {
        runtime.swap_provider(configuration.provider);
        let unknown_policy_tools = runtime.apply_persisted_tool_policy(&configuration.tool_policy);
        for stable_id in &unknown_policy_tools {
            tracing::warn!(target: "reload", stable_id = %stable_id, "tools.policy reload entry did not match a registered tool");
        }
        ReloadOutcome::Reloaded {
            unknown_policy_tools,
        }
    }
}

impl std::fmt::Debug for ReloadRuntimeConfiguration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReloadRuntimeConfiguration")
            .field("configured", &self.source.is_some())
            .finish()
    }
}

#[cfg(test)]
#[path = "reload_runtime_configuration_tests.rs"]
mod tests;
