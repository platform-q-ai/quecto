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
//!
//! Two phases: the rebuild ([`ReloadRuntimeConfiguration::rebuild`] /
//! [`ReloadRuntimeConfiguration::rebuild_if_changed`]) reads and composes
//! and borrows no runtime, so the interface runs it off its scheduler; the
//! apply ([`ReloadRuntimeConfiguration::apply`]) swaps the result into the
//! running session. This use case knows no scheduler.

use std::sync::Mutex;

use crate::application::catalogue::dto::ReloadOutcome;
use crate::application::catalogue::ports::{
    ReloadRuntime, ReloadedConfiguration, RuntimeConfigurationSource,
};

/// The result of a reload's rebuild phase, before anything is applied:
/// the use case decides it without touching the running session, so a
/// caller may run the phase wherever blocking work belongs and apply the
/// step on the session afterwards.
#[derive(Debug)]
pub enum ReloadStep {
    /// The run has no reloadable configuration.
    NotConfigured,
    /// Nothing to apply: no source changed, or a polled rebuild failed and
    /// the last-good runtime stays (logged).
    Unchanged,
    /// A forced rebuild failed; reported to the requester.
    Failed(String),
    /// A rebuilt configuration ready to apply.
    Ready(ReloadedConfiguration),
}

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

    /// Forced reload (UDS `reload`): [`rebuild`](Self::rebuild) then
    /// [`apply`](Self::apply) in one go.
    pub fn execute(&self, runtime: &mut dyn ReloadRuntime) -> ReloadOutcome {
        self.apply(runtime, self.rebuild())
    }

    /// Poll (before a prompt or `set_model`):
    /// [`rebuild_if_changed`](Self::rebuild_if_changed) then
    /// [`apply`](Self::apply) in one go.
    pub fn execute_if_changed(&self, runtime: &mut dyn ReloadRuntime) -> ReloadOutcome {
        self.apply(runtime, self.rebuild_if_changed())
    }

    /// The forced rebuild phase: rebuild regardless of whether anything
    /// changed; a failure is reported to the requester. Holds the source's
    /// lock for the rebuild and borrows no runtime, so it may run on a
    /// blocking thread.
    pub fn rebuild(&self) -> ReloadStep {
        let Some(source) = &self.source else {
            return ReloadStep::NotConfigured;
        };
        let mut source = source
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match source.rebuild() {
            Ok(configuration) => ReloadStep::Ready(configuration),
            Err(error) => {
                tracing::warn!(target: "reload", error = %error, "forced reload failed; keeping last-good");
                ReloadStep::Failed(error)
            }
        }
    }

    /// The polled rebuild phase: rebuild only when a watched file changed;
    /// a rebuild failure keeps the last-good runtime silently for the
    /// caller (it is logged) so the turn proceeds. Borrows no runtime.
    pub fn rebuild_if_changed(&self) -> ReloadStep {
        let Some(source) = &self.source else {
            return ReloadStep::NotConfigured;
        };
        let mut source = source
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !source.changed() {
            return ReloadStep::Unchanged;
        }
        match source.rebuild() {
            Ok(configuration) => ReloadStep::Ready(configuration),
            Err(error) => {
                tracing::warn!(target: "reload", error = %error, "reload rebuild failed; keeping last-good");
                ReloadStep::Unchanged
            }
        }
    }

    /// The apply phase: a ready configuration swaps the provider first,
    /// then the policy baseline from the same read; every other step
    /// touches nothing and reports itself.
    pub fn apply(&self, runtime: &mut dyn ReloadRuntime, step: ReloadStep) -> ReloadOutcome {
        let configuration = match step {
            ReloadStep::NotConfigured => return ReloadOutcome::NotConfigured,
            ReloadStep::Unchanged => return ReloadOutcome::Unchanged,
            ReloadStep::Failed(error) => return ReloadOutcome::Failed(error),
            ReloadStep::Ready(configuration) => configuration,
        };
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
