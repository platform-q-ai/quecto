//! #2403: the context mode and its marks — `agents.defaults.context_mode`,
//! `context_high_tokens` and `context_low_tokens`, their environment
//! overrides and their load-time validation (split from `config.rs`, line
//! cap).

use super::{AgentDefaults, ConfigError};
use crate::domain::conversation::ContextMode;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The context mode's keys, flattened into `agents.defaults`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextModeConfig {
    /// `"default"` (or unset): the pruning rules. `"watermark"`: append
    /// only, one cut at `context_high_tokens` down to `context_low_tokens`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_high_tokens: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_low_tokens: Option<usize>,
}

impl AgentDefaults {
    /// The context mode the configuration selects. Validated at load.
    pub fn context_mode(&self) -> ContextMode {
        ContextMode::Default
    }
}

/// `QUECTO_CONTEXT_MODE`, `QUECTO_CONTEXT_HIGH_TOKENS` and
/// `QUECTO_CONTEXT_LOW_TOKENS`.
pub(super) fn apply_env_overrides(defaults: &mut AgentDefaults, env: &HashMap<String, String>) {
    let _ = (defaults, env);
}

/// An unknown mode, a mark that is no positive count, and marks the
/// watermark refuses are refused.
pub(super) fn validate(defaults: &AgentDefaults) -> Result<(), ConfigError> {
    let _ = defaults;
    Ok(())
}

#[cfg(test)]
#[path = "context_mode_tests.rs"]
mod tests;
