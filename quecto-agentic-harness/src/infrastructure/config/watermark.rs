//! SPIKE (spike/watermark-context): the watermark context's switch and
//! marks. Read beside the other context dials, so a config key follows
//! the environment variables with no new plumbing.

use super::{AgentDefaults, ConfigError};
use crate::domain::conversation::watermark::ContextWatermark;
use std::collections::HashMap;

/// The one mode value that switches the watermark context on.
const WATERMARK: &str = "watermark";
/// The default pruning (the same as leaving the mode unset).
const DEFAULT: &str = "default";
const DEFAULT_HIGH_TOKENS: usize = 100_000;
const DEFAULT_LOW_TOKENS: usize = 30_000;

impl AgentDefaults {
    /// The watermark marks when `context_mode` is `watermark`; `None`
    /// (today's pruning) otherwise. Validated at load: low under high.
    pub fn context_watermark(&self) -> Option<ContextWatermark> {
        match self.context_mode.as_deref() {
            Some(WATERMARK) => ContextWatermark::new(
                self.context_high_tokens.unwrap_or(DEFAULT_HIGH_TOKENS),
                self.context_low_tokens.unwrap_or(DEFAULT_LOW_TOKENS),
            ),
            Some(_) | None => None,
        }
    }
}

/// `QUECTO_CONTEXT_MODE`, `QUECTO_CONTEXT_HIGH_TOKENS` and
/// `QUECTO_CONTEXT_LOW_TOKENS`; a mark that is no count is ignored.
pub(super) fn apply_env_overrides(defaults: &mut AgentDefaults, env: &HashMap<String, String>) {
    if let Some(mode) = env.get("QUECTO_CONTEXT_MODE") {
        defaults.context_mode = Some(mode.clone());
    }
    if let Some(n) = env
        .get("QUECTO_CONTEXT_HIGH_TOKENS")
        .and_then(|v| v.parse::<usize>().ok())
    {
        defaults.context_high_tokens = Some(n);
    }
    if let Some(n) = env
        .get("QUECTO_CONTEXT_LOW_TOKENS")
        .and_then(|v| v.parse::<usize>().ok())
    {
        defaults.context_low_tokens = Some(n);
    }
}

/// An unknown mode, or marks with low not under high, are refused.
pub(super) fn validate(defaults: &AgentDefaults) -> Result<(), ConfigError> {
    match defaults.context_mode.as_deref() {
        None | Some(DEFAULT) => Ok(()),
        Some(WATERMARK) if defaults.context_watermark().is_some() => Ok(()),
        Some(WATERMARK) => Err(ConfigError::ContextBudget(
            "context_low_tokens must be at least 1 and below context_high_tokens".to_string(),
        )),
        Some(other) => Err(ConfigError::ContextBudget(format!(
            "context_mode must be \"default\" or \"watermark\", not {other:?}"
        ))),
    }
}
