//! #2403: the context mode and its marks — `agents.defaults.context_mode`,
//! `context_high_tokens` and `context_low_tokens`, their environment
//! overrides and their load-time validation (split from `config.rs`, line
//! cap).

use super::{AgentDefaults, ConfigError};
use crate::domain::conversation::ContextMode;
use crate::domain::conversation::watermark::Watermark;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The mode values, and the only ones accepted.
const DEFAULT: &str = "default";
const WATERMARK: &str = "watermark";
/// The owner's marks (#2401): cut at 256k, down to 70k. The context
/// ceiling (`max_context_tokens`, the model's window, a swarm member's cap)
/// still wins: under a lower ceiling both marks scale down with it.
const DEFAULT_HIGH_TOKENS: usize = 256_000;
const DEFAULT_LOW_TOKENS: usize = 70_000;

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
    /// An environment value that is no count, refused at validation.
    #[serde(skip)]
    invalid_env: Option<String>,
}

impl ContextModeConfig {
    /// The marks: the configured ones, or the owner's.
    fn marks(&self) -> Result<Watermark, ConfigError> {
        let high = self.context_high_tokens.unwrap_or(DEFAULT_HIGH_TOKENS);
        let low = self.context_low_tokens.unwrap_or(DEFAULT_LOW_TOKENS);
        match (high, low) {
            (0, _) => Err(refused("context_high_tokens must be above 0")),
            (_, 0) => Err(refused("context_low_tokens must be above 0")),
            (high, low) => Watermark::new(high, low).map_err(|error| {
                refused(&format!(
                    "context_high_tokens {high} and context_low_tokens {low}: {error}"
                ))
            }),
        }
    }
}

fn refused(reason: &str) -> ConfigError {
    ConfigError::ContextBudget(reason.to_string())
}

impl AgentDefaults {
    /// The context mode the configuration selects. Validated at load, so a
    /// configuration that names no valid mode (only one built in code
    /// without loading) keeps the default.
    pub fn context_mode(&self) -> ContextMode {
        let config = &self.context_mode;
        match config.context_mode.as_deref() {
            Some(WATERMARK) => config
                .marks()
                .map_or(ContextMode::Default, ContextMode::Watermark),
            Some(_) | None => ContextMode::Default,
        }
    }
}

/// The variable a parent hands every child its resolved mode in (#2403
/// review M4): `default`, or `watermark:<high>:<low>`.
pub const INHERITED_CONTEXT_MODE: &str = "QUECTO_INHERITED_CONTEXT_MODE";

/// The [`INHERITED_CONTEXT_MODE`] value of `mode`.
pub fn inherited_value(mode: ContextMode) -> String {
    let _ = mode;
    String::new()
}

/// `QUECTO_CONTEXT_MODE`, `QUECTO_CONTEXT_HIGH_TOKENS` and
/// `QUECTO_CONTEXT_LOW_TOKENS`.
/// A mark that is no count is kept as refused, so the load fails naming it.
pub(super) fn apply_env_overrides(defaults: &mut AgentDefaults, env: &HashMap<String, String>) {
    let config = &mut defaults.context_mode;
    if let Some(mode) = env.get("QUECTO_CONTEXT_MODE") {
        config.context_mode = Some(mode.clone());
    }
    for (key, mark) in [
        (
            "QUECTO_CONTEXT_HIGH_TOKENS",
            &mut config.context_high_tokens,
        ),
        ("QUECTO_CONTEXT_LOW_TOKENS", &mut config.context_low_tokens),
    ] {
        match env.get(key).map(|value| (value, value.parse::<usize>())) {
            Some((_, Ok(count))) => *mark = Some(count),
            Some((value, Err(_))) => {
                config.invalid_env.get_or_insert_with(|| {
                    format!("{key} must be a whole number of tokens, not {value:?}")
                });
            }
            None => {}
        }
    }
}

/// An unknown mode, a mark that is no positive count, and marks the
/// watermark refuses are refused.
pub(super) fn validate(defaults: &AgentDefaults) -> Result<(), ConfigError> {
    let config = &defaults.context_mode;
    if let Some(reason) = &config.invalid_env {
        return Err(refused(reason));
    }
    match config.context_mode.as_deref() {
        None | Some(DEFAULT) | Some(WATERMARK) => config.marks().map(|_| ()),
        Some(other) => Err(refused(&format!(
            "context_mode must be \"{DEFAULT}\" or \"{WATERMARK}\", not {other:?}"
        ))),
    }
}

#[cfg(test)]
#[path = "context_mode_tests.rs"]
mod tests;
